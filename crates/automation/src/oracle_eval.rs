//! Oracle evaluations on Home: `uv run --with <helper wheel> <file>` with an environment built from scratch (PATH, a
//! temporary HOME, uv's folders under Home's data folder and the real tokens of the oracle's tools), at most
//! `SLOTS` at once, one at a time per (file, parameters, cursor) with joining callers, a fresh result reused within
//! `min_interval` (or a CLI caller's `max_age`), killed at the timeout (its whole process group).
//!
//! Every evaluation is recorded (`oracle_eval`) with its callers and what came of it for each (`set_consequence`).
//! Cursors belong to consumers (`oracle_cursor`): an evaluation only proposes one, the consumer commits it when
//! it acts (`commit`, or `commit_in` inside the consumer's own claim). Each oracle keeps its failure streak
//! (`oracle_streak`) for its Today row. A Home restart drops evaluations in flight: they only read.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use launchpad_core::App;
use launchpad_core::exec::{Cmd, Io};
use launchpad_core::shellenv::UserEnv;
use launchpad_core::util::now_ms;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use ts_rs::TS;

use crate::oracles::{self, LaunchpadRead, MAX_CURSOR, MAX_EVIDENCE, Meta};

/// Evaluations running at once on Home.
pub const SLOTS: usize = 16;
/// How long uv may take to prepare an environment (resolve and install) before the oracle's timeout starts.
pub const PREPARE_MS: i64 = 120_000;
/// Evaluations are kept this long, and at most `KEEP_PER_ORACLE` per oracle.
const KEEP_MS: i64 = 30 * 86_400_000;
const KEEP_PER_ORACLE: i64 = 2000;
/// Bytes of stderr kept with an evaluation, and of stdout read at most.
const STDERR_TAIL: usize = 4096;
const STDOUT_MAX: usize = 1 << 20;
/// The line the helper writes to stderr once it is imported: uv prepared the environment.
const START_MARK: &str = "\u{1e}launchpad_oracle:start";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "OracleOutcome")]
pub enum Outcome {
    True,
    False,
    Failed,
}

/// Why an evaluation failed: drives grouping in Today and what to fix.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "OracleErrorClass")]
pub enum ErrorClass {
    /// A host ctx.http refuses, a tool not granted to the workspace or without its token.
    Access,
    /// An HTTP error the snippet raised or did not handle (status, DNS, TLS, timeout of a request).
    Http,
    /// Killed at the oracle's timeout.
    Timeout,
    /// uv could not prepare the environment (resolution, install, Python).
    Env,
    /// An uncaught exception.
    Script,
    /// No result, an invalid one, or a cursor or evidence over its cap.
    Contract,
    /// The snippet called failed(reason).
    Declared,
}

impl ErrorClass {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorClass::Access => "access",
            ErrorClass::Http => "http",
            ErrorClass::Timeout => "timeout",
            ErrorClass::Env => "env",
            ErrorClass::Script => "script",
            ErrorClass::Contract => "contract",
            ErrorClass::Declared => "declared",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[ts(rename = "OracleError")]
pub struct EvalError {
    pub class: ErrorClass,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "OracleCallerKind")]
pub enum CallerKind {
    /// A routine's gate (id: the routine).
    Routine,
    /// A sleeping agent's oracle condition (Phase 2; id: the session).
    Sleep,
    /// `launchpad oracle run` (id: the session).
    Cli,
    /// The owner (Test, Run now).
    Owner,
}

/// Who asked for an evaluation.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[ts(rename = "OracleCaller")]
pub struct Caller {
    pub kind: CallerKind,
    pub id: Option<String>,
    /// For display: the routine's title, the session's title.
    pub name: Option<String>,
}

/// One caller of an evaluation (the one that started it, those that joined it in flight or reused it) and what
/// came of it for that caller.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(rename = "OracleCallerUse")]
pub struct CallerUse {
    pub caller: Caller,
    #[ts(type = "number")]
    pub at: i64,
    /// `started`, `joined` (in flight) or `reused` (a fresh result).
    pub how: String,
    /// Set by the consumer: `fired`, `quiet`, `coalesced`, `counted` (in the Today row), `skipped`, …
    pub consequence: Option<String>,
    /// A detail of the consequence: the run, the reason.
    pub detail: Option<String>,
}

/// One evaluation.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(rename = "OracleEvaluation")]
pub struct Evaluation {
    pub id: String,
    pub workspace_id: String,
    pub oracle: String,
    /// Hash of the file it ran.
    pub file_sha: String,
    #[ts(type = "Record<string, unknown>")]
    pub params: Value,
    #[ts(type = "Record<string, unknown>")]
    pub event: Value,
    /// The consumer whose cursor it read (None: stateless).
    pub cursor_key: Option<String>,
    #[ts(type = "unknown")]
    pub cursor_in: Option<Value>,
    /// The cursor the oracle proposes (None: keep the committed one). Committed only by the consumer.
    #[ts(type = "unknown")]
    pub cursor_proposed: Option<Value>,
    pub outcome: Outcome,
    pub error: Option<EvalError>,
    pub summary: String,
    pub fingerprint: Option<String>,
    pub links: Vec<String>,
    #[ts(type = "unknown[]")]
    pub items: Vec<Value>,
    pub stderr_tail: String,
    #[ts(type = "number")]
    pub duration_ms: i64,
    #[ts(type = "number")]
    pub created_at: i64,
    pub callers: Vec<CallerUse>,
    /// This answer reused an earlier or concurrent evaluation (not stored).
    #[serde(default)]
    pub reused: bool,
}

/// What a consumer asks for.
#[derive(Clone, Debug)]
pub struct Request {
    pub workspace_id: String,
    pub oracle: String,
    /// Given parameters (strings from `--param` or typed values), checked with `oracles::validate_params`.
    pub params: Map<String, Value>,
    pub caller: Caller,
    /// `{"kind": "schedule" | "cli" | "owner", ...}`: why it runs (data, never instructions).
    pub event: Value,
    /// The consumer's cursor key (`routine:<id>`, `cli:<session or name>`…): its committed cursor is the input.
    pub cursor_key: Option<String>,
    /// The cursor of a stateless caller (`--since`), when `cursor_key` is None.
    pub cursor: Option<Value>,
    /// A CLI caller accepts a result this fresh.
    pub max_age_ms: Option<i64>,
    /// The owner's Test: never reuse a recent result (still joins one in flight).
    pub force: bool,
}

/// What the helper reads on stdin.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Input {
    pub params: Value,
    pub event: Value,
    pub cursor: Option<Value>,
    /// Milliseconds since the epoch.
    pub now: i64,
    /// Hosts ctx.http may call (`*`, `*.example.com`).
    pub hosts: Vec<String>,
    /// Seconds.
    pub timeout: f64,
    /// `{"url", "token"}` with Launchpad read access.
    pub launchpad: Option<Value>,
}

impl Input {
    /// An input with these parameters, event and cursor (no hosts, now, 10 s).
    pub fn new(params: Value, event: Value, cursor: Option<Value>) -> Input {
        Input { params, event, cursor, now: now_ms(), hosts: vec![], timeout: oracles::DEFAULT_TIMEOUT_MS as f64 / 1000.0, launchpad: None }
    }
}

/// The result of one `uv run`, without records.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(rename = "OracleRunResult")]
pub struct RunResult {
    pub outcome: Outcome,
    pub error: Option<EvalError>,
    pub summary: String,
    pub fingerprint: Option<String>,
    pub links: Vec<String>,
    #[ts(type = "unknown[]")]
    pub items: Vec<Value>,
    #[ts(type = "unknown")]
    pub cursor: Option<Value>,
    pub stderr_tail: String,
    #[ts(type = "number")]
    pub duration_ms: i64,
}

impl RunResult {
    fn failed(class: ErrorClass, message: impl Into<String>) -> RunResult {
        RunResult { outcome: Outcome::Failed, error: Some(EvalError { class, message: message.into() }), summary: String::new(), fingerprint: None, links: vec![], items: vec![], cursor: None, stderr_tail: String::new(), duration_ms: 0 }
    }
}

// ---------------------------------------------------------------- one run (no App: Home and `oracle test`)

/// Runs an oracle file once: `uv run --no-project --with <wheel> --script <file>` in `cwd`, with exactly `env`
/// (it must hold PATH and HOME), the input on stdin, killed (process group) after `PREPARE_MS` without the
/// helper's start, or `input.timeout` after it.
pub async fn run_file(file: &Path, wheel: &Path, cwd: &Path, input: &Input, env: BTreeMap<String, String>) -> RunResult {
    let started = Instant::now();
    let mut r = run_inner(file, wheel, cwd, input, env).await;
    r.duration_ms = started.elapsed().as_millis() as i64;
    r
}

async fn run_inner(file: &Path, wheel: &Path, cwd: &Path, input: &Input, env: BTreeMap<String, String>) -> RunResult {
    let user = UserEnv::from_vars(env);
    if user.which("uv").is_none() {
        return RunResult::failed(ErrorClass::Env, "uv is not installed on this machine (oracles run with uv run)");
    }
    let mut cmd = Cmd::new(&user, "uv", Some(cwd));
    cmd.args(["run", "--no-project", "--quiet", "--with"]).arg(wheel).arg("--script").arg(file);
    cmd.process_tree().stdin(Io::Piped).stdout(Io::Piped).stderr(Io::Piped);
    let mut proc = match cmd.spawn().await {
        Ok(p) => p,
        Err(e) => return RunResult::failed(ErrorClass::Env, format!("could not start uv: {e}")),
    };
    if let Some(mut w) = proc.stdin.take() {
        let data = serde_json::to_vec(input).unwrap_or_default();
        tokio::spawn(async move {
            let _ = w.write_all(&data).await;
            let _ = w.shutdown().await;
        });
    }
    let (start_tx, mut start_rx) = tokio::sync::watch::channel(false);
    let mut stdout = proc.stdout.take();
    let out_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        if let Some(r) = stdout.as_mut() {
            let mut chunk = [0u8; 8192];
            while let Ok(n) = r.read(&mut chunk).await {
                if n == 0 {
                    break;
                }
                if buf.len() < STDOUT_MAX {
                    buf.extend_from_slice(&chunk[..n]);
                }
            }
        }
        buf
    });
    let mut stderr = proc.stderr.take();
    let err_task = tokio::spawn(async move {
        let mut tail: Vec<u8> = Vec::new();
        let mut seen = false;
        if let Some(r) = stderr.as_mut() {
            let mut chunk = [0u8; 8192];
            while let Ok(n) = r.read(&mut chunk).await {
                if n == 0 {
                    break;
                }
                tail.extend_from_slice(&chunk[..n]);
                if !seen && String::from_utf8_lossy(&tail).contains(START_MARK) {
                    seen = true;
                    let _ = start_tx.send(true);
                }
                if tail.len() > 4 * STDERR_TAIL {
                    tail.drain(..tail.len() - 2 * STDERR_TAIL);
                }
            }
        }
        tail
    });
    let timeout = Duration::from_millis((input.timeout * 1000.0) as u64);
    let mut deadline = tokio::time::Instant::now() + Duration::from_millis(PREPARE_MS as u64);
    let mut began = false;
    let mut killed = None;
    loop {
        tokio::select! {
            _ = proc.wait() => break,
            changed = start_rx.changed(), if !began => {
                if changed.is_ok() && *start_rx.borrow() {
                    began = true;
                    deadline = tokio::time::Instant::now() + timeout;
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                let _ = proc.kill().await;
                killed = Some(if began {
                    EvalError { class: ErrorClass::Timeout, message: format!("no answer within {} s: killed", timeout.as_secs_f64()) }
                } else {
                    EvalError { class: ErrorClass::Env, message: format!("uv did not prepare the environment within {} s", PREPARE_MS / 1000) }
                });
                break;
            }
        }
    }
    let collect = |t: tokio::task::JoinHandle<Vec<u8>>| async move { tokio::time::timeout(Duration::from_secs(2), t).await.ok().and_then(|r| r.ok()).unwrap_or_default() };
    let (out, err) = (collect(out_task).await, collect(err_task).await);
    let stderr_tail = tail_text(&err);
    let began = began || String::from_utf8_lossy(&err).contains(START_MARK);
    let mut r = match killed {
        Some(e) => RunResult::failed(e.class, e.message),
        None => parse_output(&String::from_utf8_lossy(&out), &stderr_tail, began),
    };
    r.stderr_tail = stderr_tail;
    r
}

/// The end of stderr, without the helper's start line.
fn tail_text(err: &[u8]) -> String {
    let text = String::from_utf8_lossy(err);
    let lines: Vec<&str> = text.lines().filter(|l| !l.contains(START_MARK)).collect();
    let joined = lines.join("\n");
    let cut = joined.len().saturating_sub(STDERR_TAIL);
    let start = (cut..=joined.len()).find(|i| joined.is_char_boundary(*i)).unwrap_or(joined.len());
    joined[start..].trim().to_string()
}

/// The result from the last line of stdout, checked: an outcome, a known error class, a cursor and evidence
/// within their caps. Without one, why: a traceback (script), uv's own error (env), or no `run(check)` (contract).
pub fn parse_output(stdout: &str, stderr_tail: &str, began: bool) -> RunResult {
    let last = stdout.lines().rev().map(str::trim).find(|l| !l.is_empty());
    let Some(v) = last.and_then(|l| serde_json::from_str::<Value>(l).ok()).filter(|v| v.get("outcome").is_some()) else {
        let last_line = stderr_tail.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").to_string();
        if stderr_tail.contains("Traceback (most recent call last)") {
            return RunResult::failed(ErrorClass::Script, if last_line.is_empty() { "the oracle raised an exception".to_string() } else { last_line });
        }
        if !began {
            // uv's own error, on one line.
            let text: String = stderr_tail.lines().map(|l| l.trim().trim_start_matches(['×', '╰', '─', '▶', ' '])).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ").chars().take(500).collect();
            return RunResult::failed(ErrorClass::Env, if text.is_empty() { "uv could not run the oracle".to_string() } else { text });
        }
        return RunResult::failed(ErrorClass::Contract, "the oracle wrote no result: does the file end with run(check)?");
    };
    let outcome = match v["outcome"].as_str() {
        Some("true") => Outcome::True,
        Some("false") => Outcome::False,
        Some("failed") => {
            let class = serde_json::from_value(v["error"]["class"].clone()).unwrap_or(ErrorClass::Contract);
            let message: String = v["error"]["message"].as_str().unwrap_or("failed").chars().take(2000).collect();
            return RunResult::failed(class, message);
        }
        _ => return RunResult::failed(ErrorClass::Contract, "the result's outcome is not true, false or failed"),
    };
    let cursor = v.get("cursor").cloned().filter(|c| !c.is_null());
    if cursor.as_ref().is_some_and(|c| c.to_string().len() > MAX_CURSOR) {
        return RunResult::failed(ErrorClass::Contract, format!("the cursor is larger than {MAX_CURSOR} bytes"));
    }
    let summary = v["summary"].as_str().unwrap_or("").to_string();
    let fingerprint = v["fingerprint"].as_str().map(str::to_string);
    let links: Vec<String> = v["links"].as_array().map(|a| a.iter().filter_map(|l| l.as_str().map(str::to_string)).collect()).unwrap_or_default();
    let items = v["items"].as_array().cloned().unwrap_or_default();
    if serde_json::json!([summary, fingerprint, links, items]).to_string().len() > MAX_EVIDENCE {
        return RunResult::failed(ErrorClass::Contract, format!("the evidence is larger than {MAX_EVIDENCE} bytes"));
    }
    RunResult { outcome, error: None, summary, fingerprint, links, items, cursor, stderr_tail: String::new(), duration_ms: 0 }
}

/// Hosts ctx.http may call: the oracle's own, then its tools' (the hosts that receive their tokens).
pub fn hosts_for(meta: &Meta, tools: &[launchpad_core::vault::Tool]) -> Vec<String> {
    let mut hosts = meta.access.hosts.clone();
    for t in tools.iter().filter(|t| meta.access.tools.contains(&t.id)) {
        hosts.extend(t.hosts.iter().map(|h| h.host.to_ascii_lowercase()));
    }
    hosts.dedup();
    hosts
}

/// `launchpad oracle test` on a runner: runs a draft file with this process's helper wheel (written into
/// `wheel_dir`), in a scratch folder, with `env` as given (the agent's own environment and tokens). Records
/// nothing.
pub async fn test_file(file: &Path, wheel_dir: &Path, params: Map<String, Value>, cursor: Option<Value>, hosts: Vec<String>, env: BTreeMap<String, String>) -> Result<RunResult> {
    let content = std::fs::read_to_string(file).with_context(|| format!("read {}", file.display()))?;
    let meta = oracles::parse(&content)?;
    let params = oracles::validate_params(&meta.params, &params)?;
    let wheel = crate::oracle_helper::write_wheel(wheel_dir)?;
    let cwd = scratch()?;
    let file = std::fs::canonicalize(file)?;
    let input = Input { params: Value::Object(params), event: serde_json::json!({"kind": "test"}), cursor, now: now_ms(), hosts, timeout: meta.timeout_ms as f64 / 1000.0, launchpad: None };
    let r = run_file(&file, &wheel, &cwd, &input, env).await;
    let _ = std::fs::remove_dir_all(&cwd);
    Ok(r)
}

fn scratch() -> Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!("launchpad-oracle-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

// ---------------------------------------------------------------- Home

fn ensure_home(app: &App) -> Result<()> {
    ensure!(launchpad_core::home_git::here(app), "oracles run on Home");
    Ok(())
}

fn data(app: &App) -> PathBuf {
    app.paths.data_dir.join("oracles")
}

/// The process environment of an evaluation on Home, built from scratch: PATH, HOME, uv's folders, and the real
/// token of each listed tool under its variables. A tool that is unknown, not granted to the workspace or without
/// its token fails the evaluation (class access), like Launchpad read access (not available yet).
async fn environment(app: &App, ws: &str, meta: &Meta, home: &Path) -> std::result::Result<(BTreeMap<String, String>, Vec<String>), EvalError> {
    let access = |m: String| EvalError { class: ErrorClass::Access, message: m };
    if meta.access.launchpad == LaunchpadRead::Read {
        return Err(access("launchpad = \"read\" is not available yet: remove it from access".into()));
    }
    let base = data(app);
    let mut env = BTreeMap::new();
    env.insert("PATH".to_string(), app.env.get("PATH").unwrap_or("/usr/local/bin:/usr/bin:/bin").to_string());
    env.insert("HOME".into(), home.display().to_string());
    env.insert("UV_CACHE_DIR".into(), base.join("uv-cache").display().to_string());
    env.insert("UV_PYTHON_INSTALL_DIR".into(), base.join("uv-python").display().to_string());
    env.insert("LANG".into(), "C.UTF-8".into());
    env.insert("PYTHONUNBUFFERED".into(), "1".into());
    let tools = launchpad_core::vault::tools(app).await.map_err(|e| access(format!("Home's tool registry: {e:#}")))?;
    for id in &meta.access.tools {
        let t = tools.iter().find(|t| &t.id == id).ok_or_else(|| access(format!("there is no tool {id} in Home's registry")))?;
        if !t.granted(Some(ws)) {
            return Err(access(format!("the tool {} is not granted to this workspace (Settings → Credentials and tools)", t.name)));
        }
        if let Some(cid) = &t.credential {
            let secret = match launchpad_core::vault::secret(app, cid).await {
                Ok(Some((launchpad_core::vault::Kind::Token, s))) => s,
                _ => return Err(access(format!("the tool {} has no token in Home's vault (Settings → Credentials and tools)", t.name))),
            };
            for var in &t.env {
                env.insert(var.clone(), secret.clone());
            }
        }
    }
    Ok((env, hosts_for(meta, &tools)))
}

/// Runs content on Home (the owner's Test of a draft, or an evaluation), in a fresh temporary HOME.
/// `input` holds the parameters, event and cursor; the hosts, time and timeout are set here.
async fn run_on_home(app: &App, ws: &str, slug: &str, content: &str, meta: &Meta, mut input: Input) -> RunResult {
    let home = match scratch() {
        Ok(h) => h,
        Err(e) => return RunResult::failed(ErrorClass::Env, format!("temporary folder: {e:#}")),
    };
    let r = async {
        let (env, hosts) = match environment(app, ws, meta, &home).await {
            Ok(x) => x,
            Err(e) => return RunResult::failed(e.class, e.message),
        };
        let wheel = match crate::oracle_helper::write_wheel(&data(app)) {
            Ok(w) => w,
            Err(e) => return RunResult::failed(ErrorClass::Env, format!("the launchpad_oracle wheel: {e:#}")),
        };
        let file = home.join(format!("{slug}.py"));
        if let Err(e) = std::fs::write(&file, content) {
            return RunResult::failed(ErrorClass::Env, format!("temporary file: {e}"));
        }
        (input.now, input.hosts, input.timeout) = (now_ms(), hosts, meta.timeout_ms as f64 / 1000.0);
        let _slot = slots().acquire().await;
        run_file(&file, &wheel, &home, &input, env).await
    }
    .await;
    let _ = tokio::fs::remove_dir_all(&home).await;
    r
}

fn slots() -> &'static tokio::sync::Semaphore {
    static SLOTS_SEM: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    SLOTS_SEM.get_or_init(|| tokio::sync::Semaphore::new(SLOTS))
}

type Flight = tokio::sync::watch::Receiver<Option<std::result::Result<Arc<Evaluation>, String>>>;

fn flights() -> &'static Mutex<HashMap<String, Flight>> {
    static FLIGHTS: OnceLock<Mutex<HashMap<String, Flight>>> = OnceLock::new();
    FLIGHTS.get_or_init(Default::default)
}

/// The single-flight and reuse key: workspace, file, parameters and input cursor.
fn flight_key(ws: &str, file_sha: &str, params: &Value, cursor: &Option<Value>) -> String {
    hex::encode(Sha256::digest(serde_json::json!([ws, file_sha, params, cursor]).to_string().as_bytes()))
}

/// The owner's Test of a draft on Home (Schedules → Oracles): the same environment and limits as an evaluation,
/// recorded nowhere. `content` must parse.
pub async fn test_draft(app: &App, ws: &str, slug: &str, content: &str, params: &Map<String, Value>, cursor: Option<Value>) -> Result<RunResult> {
    ensure_home(app)?;
    let meta = oracles::check(app, content).await?;
    let params = oracles::validate_params(&meta.params, params)?;
    Ok(run_on_home(app, ws, slug, content, &meta, Input::new(Value::Object(params), serde_json::json!({"kind": "test"}), cursor)).await)
}

/// Evaluates a saved oracle for a consumer: reuses a fresh result, joins one in flight, or runs it (one of `SLOTS`).
/// The run continues in its own task when the caller goes away. An error means it never reached the oracle (unknown,
/// paused, a file with an error, bad parameters); everything else is an `Evaluation`, Failed ones included.
pub async fn evaluate(app: &App, req: Request) -> Result<Evaluation> {
    ensure_home(app)?;
    let file = oracles::get(app, &req.workspace_id, &req.oracle).await?;
    ensure!(!file.oracle.paused, "the oracle {} is paused", req.oracle);
    let Some(meta) = file.oracle.meta.clone() else { bail!("the oracle {} has an error: {}", req.oracle, file.oracle.error.unwrap_or_default()) };
    let params = Value::Object(oracles::validate_params(&meta.params, &req.params)?);
    let cursor_in = match &req.cursor_key {
        Some(k) => cursor(app, k).await?,
        None => req.cursor.clone(),
    };
    let key = flight_key(&req.workspace_id, &file.oracle.sha256, &params, &cursor_in);
    let window = meta.min_interval_ms.max(req.max_age_ms.unwrap_or(0));
    if !req.force
        && let Some(e) = fresh(app, &key, now_ms() - window).await?
    {
        return add_caller(app, e, &req.caller, "reused").await;
    }
    let (mut rx, joined) = {
        let mut map = flights().lock().unwrap_or_else(|p| p.into_inner());
        match map.get(&key) {
            Some(rx) => (rx.clone(), true),
            None => {
                let (tx, rx) = tokio::sync::watch::channel(None);
                map.insert(key.clone(), rx.clone());
                let app2 = app.clone();
                let (req2, key2, file_sha, content) = (req.clone(), key.clone(), file.oracle.sha256.clone(), file.content.clone());
                let (meta2, params2, cursor2) = (meta.clone(), params.clone(), cursor_in.clone());
                tokio::spawn(async move {
                    let r = run_and_record(&app2, &req2, &key2, &file_sha, &content, &meta2, params2, cursor2).await;
                    flights().lock().unwrap_or_else(|p| p.into_inner()).remove(&key2);
                    let _ = tx.send(Some(r.map(Arc::new).map_err(|e| format!("{e:#}"))));
                });
                (rx, false)
            }
        }
    };
    let r = rx.wait_for(|v| v.is_some()).await.map_err(|_| anyhow::anyhow!("the evaluation stopped"))?.clone();
    let e = r.expect("checked").map_err(anyhow::Error::msg)?;
    if !joined {
        return Ok((*e).clone());
    }
    add_caller(app, (*e).clone(), &req.caller, "joined").await
}

#[allow(clippy::too_many_arguments)]
async fn run_and_record(app: &App, req: &Request, key: &str, file_sha: &str, content: &str, meta: &Meta, params: Value, cursor_in: Option<Value>) -> Result<Evaluation> {
    let r = run_on_home(app, &req.workspace_id, &req.oracle, content, meta, Input::new(params.clone(), req.event.clone(), cursor_in.clone())).await;
    let at = now_ms();
    let e = Evaluation {
        id: format!("ev_{}", uuid::Uuid::new_v4().simple()),
        workspace_id: req.workspace_id.clone(),
        oracle: req.oracle.clone(),
        file_sha: file_sha.into(),
        params,
        event: req.event.clone(),
        cursor_key: req.cursor_key.clone(),
        cursor_in,
        cursor_proposed: r.cursor,
        outcome: r.outcome,
        error: r.error,
        summary: r.summary,
        fingerprint: r.fingerprint,
        links: r.links,
        items: r.items,
        stderr_tail: r.stderr_tail,
        duration_ms: r.duration_ms,
        created_at: at,
        callers: vec![CallerUse { caller: req.caller.clone(), at, how: "started".into(), consequence: None, detail: None }],
        reused: false,
    };
    let (row, key) = (e.clone(), key.to_string());
    app.state_db
        .call(move |c| {
            let tx = c.transaction()?;
            tx.execute(
                "INSERT INTO oracle_eval (id, workspace_id, oracle, file_sha, flight_key, outcome, created_at, json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![row.id, row.workspace_id, row.oracle, row.file_sha, key, outcome_str(row.outcome), row.created_at, serde_json::to_string(&row)?],
            )?;
            let prev: Option<String> = tx.query_row("SELECT json FROM oracle_streak WHERE workspace_id = ?1 AND oracle = ?2", params![row.workspace_id, row.oracle], |r| r.get(0)).optional()?;
            let prev = prev.and_then(|j| serde_json::from_str::<Streak>(&j).ok()).unwrap_or_else(|| Streak::new(&row.workspace_id, &row.oracle));
            let next = prev.next(row.outcome, row.error.as_ref(), row.created_at);
            tx.execute(
                "INSERT INTO oracle_streak (workspace_id, oracle, json) VALUES (?1, ?2, ?3) ON CONFLICT (workspace_id, oracle) DO UPDATE SET json = ?3",
                params![row.workspace_id, row.oracle, serde_json::to_string(&next)?],
            )?;
            tx.commit()?;
            Ok(())
        })
        .await?;
    app.bus.publish("oracles", "evaluated", serde_json::json!({"workspace_id": e.workspace_id, "slug": e.oracle, "id": e.id, "outcome": e.outcome}));
    Ok(e)
}

fn outcome_str(o: Outcome) -> &'static str {
    match o {
        Outcome::True => "true",
        Outcome::False => "false",
        Outcome::Failed => "failed",
    }
}

async fn fresh(app: &App, key: &str, since: i64) -> Result<Option<Evaluation>> {
    let key = key.to_string();
    let json: Option<String> = app.state_db.call(move |c| Ok(c.query_row("SELECT json FROM oracle_eval WHERE flight_key = ?1 AND created_at >= ?2 ORDER BY created_at DESC LIMIT 1", params![key, since], |r| r.get(0)).optional()?)).await?;
    Ok(json.and_then(|j| serde_json::from_str(&j).ok()))
}

/// Adds a caller that joined or reused an evaluation.
async fn add_caller(app: &App, mut e: Evaluation, caller: &Caller, how: &str) -> Result<Evaluation> {
    let use_ = CallerUse { caller: caller.clone(), at: now_ms(), how: how.into(), consequence: None, detail: None };
    let id = e.id.clone();
    let stored = app
        .state_db
        .call(move |c| {
            let tx = c.transaction()?;
            let json: Option<String> = tx.query_row("SELECT json FROM oracle_eval WHERE id = ?1", [&id], |r| r.get(0)).optional()?;
            let Some(mut stored) = json.and_then(|j| serde_json::from_str::<Evaluation>(&j).ok()) else { return Ok(None) };
            stored.callers.push(use_);
            tx.execute("UPDATE oracle_eval SET json = ?2 WHERE id = ?1", params![id, serde_json::to_string(&stored)?])?;
            tx.commit()?;
            Ok(Some(stored))
        })
        .await?;
    if let Some(s) = stored {
        e = s;
    }
    e.reused = true;
    Ok(e)
}

/// Records what came of an evaluation for one caller (its latest use without a consequence, else its latest).
pub async fn set_consequence(app: &App, evaluation_id: &str, caller: &Caller, consequence: &str, detail: Option<String>) -> Result<()> {
    let (id, caller, consequence) = (evaluation_id.to_string(), caller.clone(), consequence.to_string());
    app.state_db
        .call(move |c| {
            let tx = c.transaction()?;
            let json: Option<String> = tx.query_row("SELECT json FROM oracle_eval WHERE id = ?1", [&id], |r| r.get(0)).optional()?;
            let Some(mut e) = json.and_then(|j| serde_json::from_str::<Evaluation>(&j).ok()) else { anyhow::bail!("no evaluation {id}") };
            let pick = e.callers.iter().rposition(|u| u.caller == caller && u.consequence.is_none()).or_else(|| e.callers.iter().rposition(|u| u.caller == caller));
            let Some(i) = pick else { anyhow::bail!("this caller did not ask for evaluation {id}") };
            e.callers[i].consequence = Some(consequence.clone());
            e.callers[i].detail = detail;
            tx.execute("UPDATE oracle_eval SET json = ?2, consequence = ?3 WHERE id = ?1", params![id, serde_json::to_string(&e)?, consequence])?;
            tx.commit()?;
            Ok(())
        })
        .await
}

/// One evaluation.
pub async fn get(app: &App, id: &str) -> Result<Evaluation> {
    let id = id.to_string();
    let json: Option<String> = app.state_db.call(move |c| Ok(c.query_row("SELECT json FROM oracle_eval WHERE id = ?1", [&id], |r| r.get(0)).optional()?)).await?;
    json.and_then(|j| serde_json::from_str(&j).ok()).context("no such evaluation")
}

/// Evaluations of a workspace (one oracle's with `oracle`), newest first: only Failed ones with `failed_only`,
/// older than `before` when given.
pub async fn history(app: &App, ws: &str, oracle: Option<&str>, failed_only: bool, before: Option<i64>, limit: usize) -> Result<Vec<Evaluation>> {
    let (ws, oracle) = (ws.to_string(), oracle.map(str::to_string));
    let rows: Vec<(String, String)> = app
        .state_db
        .read(move |c| {
            let mut st = c.prepare(
                "SELECT id, json FROM oracle_eval WHERE workspace_id = ?1 AND (?2 IS NULL OR oracle = ?2) AND (?3 = 0 OR outcome = 'failed') AND created_at < ?4
                 ORDER BY created_at DESC LIMIT ?5",
            )?;
            let rows = st.query_map(params![ws, oracle, failed_only as i64, before.unwrap_or(i64::MAX), limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await?;
    Ok(crate::decode_rows("oracle evaluation", rows, |j| Ok(serde_json::from_str(j)?)))
}

/// The last `n` outcomes of each oracle of a workspace, newest first, with their times (the list's squares).
pub async fn recent(app: &App, ws: &str, n: usize) -> Result<BTreeMap<String, Vec<(Outcome, i64)>>> {
    let ws = ws.to_string();
    let rows: Vec<(String, String, i64)> = app
        .state_db
        .read(move |c| {
            let mut st = c.prepare(
                "SELECT oracle, outcome, created_at FROM (SELECT oracle, outcome, created_at, row_number() OVER (PARTITION BY oracle ORDER BY created_at DESC) AS k
                 FROM oracle_eval WHERE workspace_id = ?1) WHERE k <= ?2 ORDER BY oracle, created_at DESC",
            )?;
            let rows = st.query_map(params![ws, n as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await?;
    let mut out: BTreeMap<String, Vec<(Outcome, i64)>> = BTreeMap::new();
    for (oracle, outcome, at) in rows {
        let o = match outcome.as_str() {
            "true" => Outcome::True,
            "false" => Outcome::False,
            _ => Outcome::Failed,
        };
        out.entry(oracle).or_default().push((o, at));
    }
    Ok(out)
}

// ---------------------------------------------------------------- cursors

/// A consumer's committed cursor.
pub async fn cursor(app: &App, consumer: &str) -> Result<Option<Value>> {
    let k = consumer.to_string();
    let json: Option<String> = app.state_db.call(move |c| Ok(c.query_row("SELECT cursor FROM oracle_cursor WHERE consumer = ?1", [&k], |r| r.get(0)).optional()?)).await?;
    Ok(json.and_then(|j| serde_json::from_str(&j).ok()))
}

/// Commits a cursor inside the caller's own transaction (a routine's claim): the claim and the cursor change
/// together.
pub fn commit_in(c: &rusqlite::Connection, consumer: &str, ws: &str, oracle: &str, cursor: &Value, evaluation_id: Option<&str>, at: i64) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO oracle_cursor (consumer, workspace_id, oracle, cursor, evaluation_id, committed_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (consumer) DO UPDATE SET workspace_id = ?2, oracle = ?3, cursor = ?4, evaluation_id = ?5, committed_at = ?6",
        params![consumer, ws, oracle, cursor.to_string(), evaluation_id, at],
    )?;
    Ok(())
}

/// Commits the cursor an evaluation proposed for a consumer. False when it proposed none (the cursor stays).
pub async fn commit(app: &App, consumer: &str, evaluation_id: &str) -> Result<bool> {
    let e = get(app, evaluation_id).await?;
    let Some(cursor) = e.cursor_proposed else { return Ok(false) };
    let consumer = consumer.to_string();
    app.state_db.call(move |c| Ok(commit_in(c, &consumer, &e.workspace_id, &e.oracle, &cursor, Some(&e.id), now_ms())?)).await?;
    Ok(true)
}

/// Forgets a consumer's cursor (a removed gate, a reset).
pub async fn forget_cursor(app: &App, consumer: &str) -> Result<()> {
    let k = consumer.to_string();
    app.state_db.call(move |c| Ok(c.execute("DELETE FROM oracle_cursor WHERE consumer = ?1", [k])?)).await?;
    Ok(())
}

// ---------------------------------------------------------------- failure streaks

/// An oracle's failure streak, for its Today row: open while `count > 0` and not read.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[ts(rename = "OracleStreak")]
pub struct Streak {
    pub workspace_id: String,
    pub oracle: String,
    /// Failed evaluations in a row (0: not failing).
    pub count: u32,
    #[ts(type = "number | null")]
    pub first_at: Option<i64>,
    #[ts(type = "number | null")]
    pub last_at: Option<i64>,
    /// Failures of this streak by class.
    pub classes: BTreeMap<String, u32>,
    pub last_error: Option<EvalError>,
    #[ts(type = "number | null")]
    pub last_good_at: Option<i64>,
    /// Mark read: hides the row until the next failure, which starts a new streak.
    #[ts(type = "number | null")]
    pub read_at: Option<i64>,
    /// The last recovery: when, and after how many failures.
    #[ts(type = "number | null")]
    pub recovered_at: Option<i64>,
    pub recovered_after: u32,
}

impl Streak {
    pub fn new(ws: &str, oracle: &str) -> Streak {
        Streak { workspace_id: ws.into(), oracle: oracle.into(), count: 0, first_at: None, last_at: None, classes: BTreeMap::new(), last_error: None, last_good_at: None, read_at: None, recovered_at: None, recovered_after: 0 }
    }

    /// Failing and not marked read: its Today row shows.
    pub fn open(&self) -> bool {
        self.count > 0 && self.read_at.is_none()
    }

    /// The most frequent error class of the streak (ties: the latest error's).
    pub fn dominant(&self) -> Option<ErrorClass> {
        let max = *self.classes.values().max()?;
        let latest = self.last_error.as_ref().map(|e| e.class);
        if let Some(l) = latest.filter(|l| self.classes.get(l.as_str()) == Some(&max)) {
            return Some(l);
        }
        let name = self.classes.iter().find(|(_, n)| **n == max)?.0.clone();
        serde_json::from_value(Value::String(name)).ok()
    }

    /// The streak after an evaluation: a failure extends it (or starts a new one when there is none or it was read),
    /// a True or False ends it as a recovery.
    pub fn next(&self, outcome: Outcome, error: Option<&EvalError>, at: i64) -> Streak {
        let mut s = self.clone();
        match outcome {
            Outcome::Failed => {
                if s.count == 0 || s.read_at.is_some() {
                    s.count = 0;
                    s.first_at = Some(at);
                    s.classes.clear();
                    s.read_at = None;
                }
                s.count += 1;
                s.last_at = Some(at);
                let class = error.map(|e| e.class).unwrap_or(ErrorClass::Script);
                *s.classes.entry(class.as_str().to_string()).or_default() += 1;
                s.last_error = error.cloned();
            }
            Outcome::True | Outcome::False => {
                if s.count > 0 {
                    s.recovered_at = Some(at);
                    s.recovered_after = s.count;
                }
                s.count = 0;
                s.read_at = None;
                s.last_good_at = Some(at);
            }
        }
        s
    }
}

/// The failure streaks of a workspace (every workspace with None).
pub async fn streaks(app: &App, ws: Option<&str>) -> Result<Vec<Streak>> {
    let ws = ws.map(str::to_string);
    let rows: Vec<(String, String)> = app
        .state_db
        .call(move |c| {
            let mut st = c.prepare("SELECT workspace_id || ':' || oracle, json FROM oracle_streak WHERE ?1 IS NULL OR workspace_id = ?1")?;
            let rows = st.query_map([ws], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await?;
    Ok(crate::decode_rows("oracle streak", rows, |j| Ok(serde_json::from_str(j)?)))
}

/// Mark read: hides the oracle's failure row until its next failure.
pub async fn mark_read(app: &App, ws: &str, oracle: &str) -> Result<()> {
    let (ws, oracle) = (ws.to_string(), oracle.to_string());
    app.state_db
        .call(move |c| {
            let json: Option<String> = c.query_row("SELECT json FROM oracle_streak WHERE workspace_id = ?1 AND oracle = ?2", params![ws, oracle], |r| r.get(0)).optional()?;
            let Some(mut s) = json.and_then(|j| serde_json::from_str::<Streak>(&j).ok()) else { return Ok(()) };
            s.read_at = Some(now_ms());
            c.execute("UPDATE oracle_streak SET json = ?3 WHERE workspace_id = ?1 AND oracle = ?2", params![ws, oracle, serde_json::to_string(&s)?])?;
            Ok(())
        })
        .await?;
    app.bus.publish("oracles", "changed", Value::Null);
    Ok(())
}

// ---------------------------------------------------------------- retention

/// Deletes evaluations older than 30 days and beyond the newest `KEEP_PER_ORACLE` of each oracle.
pub async fn prune(app: &App) -> Result<()> {
    let before = now_ms() - KEEP_MS;
    app.state_db
        .call(move |c| {
            c.execute("DELETE FROM oracle_eval WHERE created_at < ?1", [before])?;
            c.execute(
                "DELETE FROM oracle_eval WHERE id IN (SELECT id FROM (SELECT id, row_number() OVER (PARTITION BY workspace_id, oracle ORDER BY created_at DESC) AS k FROM oracle_eval) WHERE k > ?1)",
                [KEEP_PER_ORACLE],
            )?;
            Ok(())
        })
        .await
}

/// Home: prunes evaluations at start, then every day.
pub fn spawn(app: &App) {
    if !launchpad_core::home_git::here(app) {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(24 * 3600));
        loop {
            tick.tick().await;
            if let Err(e) = prune(&app).await {
                tracing::warn!("oracle evaluations retention: {e:#}");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_parsing() {
        let r = parse_output("noise\n{\"outcome\": \"true\", \"summary\": \"red\", \"fingerprint\": \"abc\", \"links\": [\"https://x\"], \"items\": [1], \"cursor\": {\"v\": 2}}\n", "", true);
        assert_eq!((r.outcome, r.summary.as_str(), r.fingerprint.as_deref(), r.cursor), (Outcome::True, "red", Some("abc"), Some(serde_json::json!({"v": 2}))));
        let r = parse_output("{\"outcome\": \"failed\", \"error\": {\"class\": \"http\", \"message\": \"401\"}}", "", true);
        assert_eq!(r.error.unwrap().class, ErrorClass::Http);
        let r = parse_output("{\"outcome\": \"failed\", \"error\": {\"class\": \"weird\"}}", "", true);
        assert_eq!(r.error.unwrap().class, ErrorClass::Contract);
        assert_eq!(parse_output("", "Traceback (most recent call last):\n  x\nKeyError: 'a'", true).error.unwrap().message, "KeyError: 'a'");
        assert_eq!(parse_output("", "error: No solution found", false).error.unwrap().class, ErrorClass::Env);
        assert_eq!(parse_output("", "", true).error.unwrap().class, ErrorClass::Contract);
        let big = format!("{{\"outcome\": \"false\", \"cursor\": \"{}\"}}", "x".repeat(MAX_CURSOR + 1));
        assert_eq!(parse_output(&big, "", true).error.unwrap().class, ErrorClass::Contract);
    }

    #[test]
    fn failure_streaks() {
        let http = EvalError { class: ErrorClass::Http, message: "401".into() };
        let s = Streak::new("w", "o").next(Outcome::Failed, Some(&http), 10).next(Outcome::Failed, Some(&http), 20);
        assert_eq!((s.count, s.first_at, s.last_at, s.open()), (2, Some(10), Some(20), true));
        assert_eq!(s.dominant(), Some(ErrorClass::Http));
        let read = Streak { read_at: Some(25), ..s.clone() };
        assert!(!read.open());
        // The next failure after Mark read starts a new streak.
        let again = read.next(Outcome::Failed, Some(&EvalError { class: ErrorClass::Timeout, message: "t".into() }), 30);
        assert_eq!((again.count, again.first_at, again.open(), again.dominant()), (1, Some(30), true, Some(ErrorClass::Timeout)));
        let ok = s.next(Outcome::False, None, 40);
        assert_eq!((ok.count, ok.recovered_at, ok.recovered_after, ok.last_good_at, ok.open()), (0, Some(40), 2, Some(40), false));
    }
}

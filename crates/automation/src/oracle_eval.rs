//! Oracle evaluations on Home: one path for every caller (gate ticks, `launchpad oracle run`, the owner's Test,
//! agents' `launchpad oracle test`). A draft is pre-checked first (`oracle_check::precheck`); a saved oracle runs as
//! stored. The file runs in a fresh Monty worker (`oracle_monty`), then `check` is called with the checked
//! arguments (plus `cursor` and `now` when it asks for them); each tool call suspends the worker and Home runs it
//! (`oracle_tools`). At most `SLOTS` at once; a saved oracle shares one evaluation per (file, parameters, cursor)
//! with joining callers and reuses an answer within `min_interval`. The secrets an evaluation used never reach its
//! result (`redact`).
//!
//! Every evaluation of a saved oracle is recorded (`oracle_eval`) with its callers, what came of it for each
//! (`set_consequence`), its log and its call log. Cursors belong to consumers (`oracle_cursor`): an evaluation only
//! proposes one, the consumer commits it when it acts (`commit`, or `commit_in` inside the consumer's own claim).
//! Each oracle keeps its failure streak (`oracle_streak`) for its Today row. A Home restart drops evaluations in
//! flight: they only read.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use launchpad_core::App;
use launchpad_core::util::now_ms;
use monty_pool::{Checkout, PoolError, ResumeValue, TurnEvent};
use monty_types::{ExcType, MontyException, MontyObject, NamedValues};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use ts_rs::TS;

use crate::oracle_monty;
use crate::oracle_tools::{CallLog, Reply, ToolBox};
use crate::oracles::{self, MAX_CURSOR, MAX_EVIDENCE, Meta, Problem};

/// Evaluations running at once on Home.
pub const SLOTS: usize = 16;
/// Evaluations are kept this long, and at most `KEEP_PER_ORACLE` per oracle.
const KEEP_MS: i64 = 30 * 86_400_000;
const KEEP_PER_ORACLE: i64 = 2000;
/// The log (`print`) kept with an evaluation.
pub const MAX_LOG: usize = 64 << 10;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq, Hash)]
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
    /// A tool missing from the header, not granted, without a token, or refused (a private address, a route outside
    /// the read level).
    Access,
    /// `raise_for_status()` on a 4xx or 5xx, a connection error, a Launchpad command error.
    Http,
    /// The evaluation ran past its timeout, tool calls included.
    Timeout,
    /// A budget ran out: memory, tool calls, response size or bytes received.
    Limit,
    /// An exception in the code (also the interpreter stopping). Evaluations of the older format recorded `env`.
    #[serde(alias = "env")]
    Script,
    /// `check` returned something other than a verdict or a bool, or broke a size limit of its evidence or cursor.
    Contract,
    /// The oracle called failed(reason).
    Declared,
}

impl ErrorClass {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorClass::Access => "access",
            ErrorClass::Http => "http",
            ErrorClass::Timeout => "timeout",
            ErrorClass::Limit => "limit",
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
    /// The call of the call log it points at (`raise_for_status()`, a tool's error).
    #[serde(default)]
    pub call: Option<usize>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "OracleCallerKind")]
pub enum CallerKind {
    /// A routine's gate (id: the routine).
    Routine,
    /// A sleeping agent's oracle condition (later; id: the session).
    Sleep,
    /// `launchpad oracle run` (id: the session).
    Cli,
    /// The owner (`oracle.run` from a window).
    Owner,
    /// The owner's Test of the saved file (`oracle.test`): recorded, commits no cursor.
    Test,
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

/// One evaluation of a saved oracle.
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
    /// Why it ran (`{"kind": "schedule" | "cli" | "owner"}`): recorded, never given to the code.
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
    /// What `print()` wrote (at most 64 KB).
    #[serde(default)]
    pub log: String,
    /// Every tool call, in order.
    #[serde(default)]
    pub calls: Vec<CallLog>,
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
    /// Given arguments (strings from `--param` or typed values), checked with `oracles::validate_params`.
    pub params: Map<String, Value>,
    pub caller: Caller,
    /// `{"kind": "schedule" | "cli" | "owner", ...}`: why it runs (recorded, never given to the code).
    pub event: Value,
    /// The consumer's cursor key (`routine:<id>`, `cli:<workspace>:<oracle>:<name>`): its committed cursor is the input.
    pub cursor_key: Option<String>,
    /// The cursor of a stateless caller (`--since`), when `cursor_key` is None.
    pub cursor: Option<Value>,
    /// A CLI caller accepts a result this fresh (never fresher than `min_interval` allows to be called again).
    pub max_age_ms: Option<i64>,
    /// The owner's Test: never reuse a recent result (still joins one in flight).
    pub force: bool,
}

/// The result of one run, without records.
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
    /// The proposed cursor (None: keep it; `null`: reset it).
    #[ts(type = "unknown")]
    pub cursor: Option<Value>,
    pub log: String,
    pub calls: Vec<CallLog>,
    #[ts(type = "number")]
    pub duration_ms: i64,
}

impl RunResult {
    pub fn failed(class: ErrorClass, message: impl Into<String>) -> RunResult {
        RunResult { outcome: Outcome::Failed, error: Some(EvalError { class, message: message.into(), call: None }), summary: String::new(), fingerprint: None, links: vec![], items: vec![], cursor: None, log: String::new(), calls: vec![], duration_ms: 0 }
    }

    fn from_error(e: EvalError) -> RunResult {
        RunResult { error: Some(e), ..RunResult::failed(ErrorClass::Script, "") }
    }
}

// ---------------------------------------------------------------- one run (Monty)

/// Pure: the answer `launchpad_oracle._finish` returns (JSON), checked: a verdict or a bool, a summary, fingerprint
/// and links of their types, items JSON, evidence within 32 KB and a cursor within 4 KB.
pub fn decide(answer: &str) -> RunResult {
    let contract = |m: String| RunResult::failed(ErrorClass::Contract, m);
    let Ok(v) = serde_json::from_str::<Value>(answer) else { return contract("check's answer could not be read".into()) };
    if v["kind"] == "contract" {
        return contract(v["message"].as_str().unwrap_or("check returned something other than a verdict").to_string());
    }
    let outcome = match v["value"] {
        Value::Bool(true) => Outcome::True,
        Value::Bool(false) => Outcome::False,
        _ => return contract(format!("verdict's value is True or False, not {}", v["value"])),
    };
    let Some(summary) = v["summary"].as_str().map(str::to_string) else { return contract("verdict's summary is a str".into()) };
    let fingerprint = match &v["fingerprint"] {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        x => return contract(format!("verdict's fingerprint is a str or None, not {x}")),
    };
    let links = match &v["links"] {
        Value::Null => vec![],
        Value::Array(a) if a.iter().all(Value::is_string) => a.iter().filter_map(|l| l.as_str().map(str::to_string)).collect(),
        _ => return contract("verdict's links is a list of str".into()),
    };
    let items = match &v["items"] {
        Value::Null => vec![],
        Value::Array(a) => a.clone(),
        _ => return contract("verdict's items is a list".into()),
    };
    let cursor = if v["cursor_set"] == Value::Bool(true) {
        match &v["cursor"] {
            Value::Null => Some(Value::Null),
            Value::String(s) if s.len() <= MAX_CURSOR => Some(Value::String(s.clone())),
            Value::String(_) => return contract(format!("the cursor is larger than {MAX_CURSOR} bytes")),
            x => return contract(format!("the cursor is a str or None, not {x}")),
        }
    } else {
        None
    };
    if serde_json::json!([summary, fingerprint, links, items]).to_string().len() > MAX_EVIDENCE {
        return contract(format!("the evidence (summary, fingerprint, links and items) is larger than {} KB", MAX_EVIDENCE >> 10));
    }
    RunResult { outcome, error: None, summary, fingerprint, links, items, cursor, log: String::new(), calls: vec![], duration_ms: 0 }
}

/// Pure: the cursor `check` receives: a committed string as is, None as None, any other committed JSON as its text.
pub fn cursor_arg(c: &Option<Value>) -> Value {
    match c {
        None | Some(Value::Null) => Value::Null,
        Some(Value::String(s)) => Value::String(s.clone()),
        Some(v) => Value::String(v.to_string()),
    }
}

/// Pure: the class and message of an exception that ended the code. An error a tool raised keeps the tool's class
/// (`raised`); the interpreter's own limits are `limit` or `timeout`; anything else is `script`, with its line.
pub fn classify(exc: &MontyException, raised: &[(ExcType, String, ErrorClass, usize)]) -> EvalError {
    let message = exc.message().unwrap_or_default();
    if let Some((_, _, class, call)) = raised.iter().rev().find(|(t, m, _, _)| *t == exc.exc_type() && m == message) {
        return EvalError { class: *class, message: exc.summary(), call: Some(*call) };
    }
    let limit = |m: String| EvalError { class: ErrorClass::Limit, message: m, call: None };
    if message.starts_with("suspension limit") {
        return limit("more tool calls and imports than an evaluation allows".into());
    }
    if message.starts_with("sleep limit exceeded") {
        return limit("time.sleep is not available: an oracle checks once and never waits".into());
    }
    if exc.exc_type() == ExcType::MemoryError {
        return limit(format!("memory: {message}"));
    }
    if exc.exc_type() == ExcType::TimeoutError && message.contains("time limit exceeded") {
        return EvalError { class: ErrorClass::Timeout, message: "the oracle ran past its timeout".into(), call: None };
    }
    let line = exc.traceback().iter().rev().find(|f| f.filename.starts_with("<python-input-")).map(|f| f.start.line);
    EvalError { class: ErrorClass::Script, message: match line { Some(l) => format!("line {l}: {}", exc.summary()), None => exc.summary() }, call: None }
}

/// What a run holds while it goes.
struct Run {
    tools: ToolBox,
    /// The errors Home raised into the code: type, message, class and call index.
    raised: Vec<(ExcType, String, ErrorClass, usize)>,
    deadline: tokio::time::Instant,
}

/// Feeds and resumes until the code completes, answering every suspension; Err ends the evaluation.
async fn drive(co: &mut Checkout, first: Result<TurnEvent, PoolError>, run: &mut Run, on_print: monty_pool::OnPrint<'_>) -> std::result::Result<MontyObject, EvalError> {
    let timeout = || EvalError { class: ErrorClass::Timeout, message: "the oracle ran past its timeout".into(), call: None };
    let mut ev = first;
    loop {
        let event = match ev {
            Ok(e) => e,
            Err(PoolError::Runtime(exc)) => return Err(classify(&exc, &run.raised)),
            Err(PoolError::Timeout { .. } | PoolError::CpuLimit { .. }) => return Err(timeout()),
            Err(PoolError::Crashed { .. }) => return Err(EvalError { class: ErrorClass::Script, message: "the interpreter stopped".into(), call: None }),
            Err(e) => return Err(EvalError { class: ErrorClass::Script, message: format!("the interpreter: {e}"), call: None }),
        };
        let answer = match event {
            TurnEvent::Complete(v) => return Ok(v),
            TurnEvent::OsCall { function_name, args, system_sleep, .. } => match system_sleep {
                Some(_) => ResumeValue::Return(MontyObject::none()),
                None => oracle_monty::os_answer(&function_name, oracle_monty::module_of(&args).as_deref()),
            },
            TurnEvent::FunctionCall { function_name, args, .. } => {
                let text = |i: usize| args.arg(i).and_then(|a| a.as_str().map(str::to_string)).unwrap_or_default();
                match function_name.as_str() {
                    "call" => match run.tools.call(&text(0), &text(1), &text(2)).await {
                        Reply::Value(v) => ResumeValue::Return(v),
                        Reply::Raise { exc, message, class } => {
                            run.raised.push((exc, message.clone(), class, run.tools.calls.len().saturating_sub(1)));
                            ResumeValue::Error(MontyException::new(exc, Some(message)))
                        }
                        Reply::Stop { class, message } => return Err(EvalError { class, message, call: run.tools.calls.len().checked_sub(1) }),
                    },
                    "fail" => {
                        let class = if text(0) == "http" { ErrorClass::Http } else { ErrorClass::Declared };
                        let call = args.arg(2).and_then(|a| a.as_int()).filter(|n| *n >= 0).map(|n| n as usize);
                        let mut message: String = text(1).chars().take(2000).collect();
                        if class == ErrorClass::Http
                            && let Some(c) = call.and_then(|c| run.tools.calls.get(c))
                        {
                            message = format!("{message} ({} {})", c.tool, c.call);
                        }
                        return Err(EvalError { class, message, call });
                    }
                    _ => ResumeValue::NotFound,
                }
            }
            TurnEvent::NameLookup { .. } => {
                ev = match tokio::time::timeout_at(run.deadline, Box::pin(co.resume_name_lookup(None::<MontyObject>, on_print))).await {
                    Ok(r) => r,
                    Err(_) => return Err(timeout()),
                };
                continue;
            }
            TurnEvent::ResolveFutures { .. } => return Err(EvalError { class: ErrorClass::Script, message: "asyncio is not available to oracles".into(), call: None }),
        };
        ev = match tokio::time::timeout_at(run.deadline, Box::pin(co.resume(answer, on_print))).await {
            Ok(r) => r,
            Err(_) => return Err(timeout()),
        };
    }
}

/// The feed that calls `check` with its arguments, then returns its answer as JSON.
const CALL: &str = "import launchpad_oracle as __launchpad_oracle\n__launchpad_oracle._finish(check, __launchpad_in)";

/// Runs an oracle file once in a fresh Monty worker: the file, then `check(**params)` (with `cursor` and `now` when
/// it asks for them). Waits for one of `SLOTS` first; the timeout counts from the start of the run, tool calls
/// included. Records nothing. Runs in its own task, so it never shares a deep stack with its caller.
pub async fn run(app: &App, ws: &str, content: &str, meta: &Meta, params: &Map<String, Value>, cursor: &Option<Value>, seed: &str) -> RunResult {
    let (app, ws, content, meta, params, cursor, seed) = (app.clone(), ws.to_string(), content.to_string(), meta.clone(), params.clone(), cursor.clone(), seed.to_string());
    tokio::spawn(async move { run_task(&app, &ws, &content, &meta, &params, &cursor, &seed).await }).await.unwrap_or_else(|_| RunResult::failed(ErrorClass::Script, "the evaluation stopped"))
}

async fn run_task(app: &App, ws: &str, content: &str, meta: &Meta, params: &Map<String, Value>, cursor: &Option<Value>, seed: &str) -> RunResult {
    let _slot = slot().await;
    let started = Instant::now();
    let now = now_ms();
    let timeout = Duration::from_millis(meta.timeout_ms as u64);
    let deadline = tokio::time::Instant::now() + timeout;
    let log = Arc::new(Mutex::new(String::new()));
    let mut run = Run { tools: ToolBox::new(app, ws, &meta.tools, started + timeout), raised: vec![], deadline };
    let mut r = match run_inner(app, content, meta, params, cursor, seed, now, &mut run, log.clone()).await {
        Ok(answer) => decide(&answer),
        Err(e) => RunResult::from_error(e),
    };
    r.log = log.lock().unwrap_or_else(|p| p.into_inner()).clone();
    r.calls = std::mem::take(&mut run.tools.calls);
    r.duration_ms = started.elapsed().as_millis() as i64;
    redact(r, &run.tools.secrets)
}

#[allow(clippy::too_many_arguments)]
async fn run_inner(app: &App, content: &str, meta: &Meta, params: &Map<String, Value>, cursor: &Option<Value>, seed: &str, now: i64, run: &mut Run, log: Arc<Mutex<String>>) -> std::result::Result<String, EvalError> {
    let script = |m: String| EvalError { class: ErrorClass::Script, message: m, call: None };
    let pool = oracle_monty::pool(app).await.map_err(|e| script(format!("{e:#}")))?;
    let session = oracle_monty::session(Duration::from_millis(meta.timeout_ms as u64), now, seed, false);
    let mut co = match tokio::time::timeout_at(run.deadline, Box::pin(pool.checkout(&session))).await {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => return Err(script(format!("the interpreter could not start: {e}"))),
        Err(_) => return Err(EvalError { class: ErrorClass::Timeout, message: "the oracle ran past its timeout".into(), call: None }),
    };
    let mut on_print = monty_pool::on_print_sync(move |_, text| {
        let mut l = log.lock().unwrap_or_else(|p| p.into_inner());
        if l.len() < MAX_LOG {
            let room = MAX_LOG - l.len();
            let cut = (0..=room.min(text.len())).rev().find(|i| text.is_char_boundary(*i)).unwrap_or(0);
            l.push_str(&text[..cut]);
        }
    });
    let first = match tokio::time::timeout_at(run.deadline, Box::pin(co.feed(content.to_string(), NamedValues::new(), vec![], true, &mut on_print))).await {
        Ok(r) => r,
        Err(_) => return Err(EvalError { class: ErrorClass::Timeout, message: "the oracle ran past its timeout".into(), call: None }),
    };
    drive(&mut co, first, run, &mut on_print).await?;
    let mut kwargs = params.clone();
    if meta.cursor {
        kwargs.insert("cursor".into(), cursor_arg(cursor));
    }
    let input = serde_json::json!({"kwargs": kwargs, "now": meta.now.then(|| now as f64 / 1000.0)});
    let mut inputs = NamedValues::new();
    inputs.push("__launchpad_in", MontyObject::string(input.to_string()));
    let second = match tokio::time::timeout_at(run.deadline, Box::pin(co.feed(CALL, inputs, vec![], true, &mut on_print))).await {
        Ok(r) => r,
        Err(_) => return Err(EvalError { class: ErrorClass::Timeout, message: "the oracle ran past its timeout".into(), call: None }),
    };
    let answer = drive(&mut co, second, run, &mut on_print).await?;
    // The worker serves this evaluation only (max one checkout per worker): dropping the checkout ends it.
    drop(co);
    answer.as_ref().as_str().map(str::to_string).ok_or_else(|| EvalError { class: ErrorClass::Contract, message: "check's answer could not be read".into(), call: None })
}

/// Pure: a result without the secrets an evaluation used (as text, also JSON-escaped, in every field), so a token
/// an API echoes never reaches records, the Today row or agents.
fn redact(r: RunResult, secrets: &[String]) -> RunResult {
    let secrets: Vec<&String> = secrets.iter().filter(|s| s.len() >= 8).collect();
    let Ok(mut text) = serde_json::to_string(&r) else { return r };
    if !secrets.iter().any(|s| text.contains(s.as_str()) || serde_json::to_string(s).is_ok_and(|j| text.contains(j.trim_matches('"')))) {
        return r;
    }
    for s in secrets {
        text = text.replace(s.as_str(), "[redacted]");
        if let Ok(j) = serde_json::to_string(s) {
            text = text.replace(j.trim_matches('"'), "[redacted]");
        }
    }
    serde_json::from_str(&text).unwrap_or_else(|_| RunResult { duration_ms: r.duration_ms, ..RunResult::failed(ErrorClass::Contract, "the result held a secret of the oracle and could not be redacted") })
}

// ---------------------------------------------------------------- Home

fn ensure_home(app: &App) -> Result<()> {
    ensure!(launchpad_core::home_git::here(app), "oracles run on Home");
    Ok(())
}

/// One of the `SLOTS` evaluations (and type checks) at once; more wait.
pub async fn slot() -> tokio::sync::SemaphorePermit<'static> {
    slots().acquire().await.expect("never closed")
}

fn slots() -> &'static tokio::sync::Semaphore {
    static SLOTS_SEM: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    SLOTS_SEM.get_or_init(|| tokio::sync::Semaphore::new(SLOTS))
}

type Answer = tokio::sync::watch::Receiver<Option<std::result::Result<Arc<Evaluation>, String>>>;

fn flights() -> &'static Mutex<HashMap<String, Answer>> {
    static FLIGHTS: OnceLock<Mutex<HashMap<String, Answer>>> = OnceLock::new();
    FLIGHTS.get_or_init(Default::default)
}

/// An evaluation in flight, by its key: removed from `flights` when dropped.
struct Flight(String);

impl Drop for Flight {
    fn drop(&mut self) {
        flights().lock().unwrap_or_else(|p| p.into_inner()).remove(&self.0);
    }
}

/// The single-flight and reuse key: workspace, file, parameters and input cursor.
fn flight_key(ws: &str, file_sha: &str, params: &Value, cursor: &Option<Value>) -> String {
    hex::encode(Sha256::digest(serde_json::json!([ws, file_sha, params, cursor]).to_string().as_bytes()))
}

/// A draft's run on Home (the owner's Test of unsaved changes, `launchpad oracle test`): pre-checked
/// (`oracle_check::Refused` when refused), then run exactly like the saved oracle would run, recorded nowhere and
/// sharing nothing. Returns the result and the type warnings.
pub async fn test_draft(app: &App, ws: &str, content: &str, params: &Map<String, Value>, cursor: Option<Value>) -> Result<(RunResult, Vec<Problem>)> {
    ensure_home(app)?;
    oracle_monty::pool(app).await?;
    let checked = crate::oracle_check::precheck(app, content).await?;
    let params = oracles::validate_params(&checked.meta.params, params)?;
    let seed = format!("draft_{}", uuid::Uuid::new_v4().simple());
    Ok((run(app, ws, &checked.content, &checked.meta, &params, &cursor, &seed).await, checked.warnings))
}

/// Evaluates a saved oracle for a consumer: reuses a fresh result, joins one in flight, or runs it (one of `SLOTS`).
/// The run continues in its own task when the caller goes away. An error means it never reached the oracle
/// (unknown, paused, broken, bad arguments, no interpreter on Home); everything else is an `Evaluation`, Failed
/// ones included.
pub async fn evaluate(app: &App, req: Request) -> Result<Evaluation> {
    ensure_home(app)?;
    let file = oracles::get(app, &req.workspace_id, &req.oracle).await?;
    ensure!(!file.oracle.paused, "the oracle {} is paused", req.oracle);
    if let Some(why) = file.oracle.broken_reason() {
        bail!("the oracle {} is broken and is not evaluated until it is saved again: {why}", req.oracle);
    }
    let meta = file.oracle.meta.clone().context("the oracle has no definition")?;
    let params = Value::Object(oracles::validate_params(&meta.params, &req.params)?);
    oracle_monty::pool(app).await?;
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
                    // Leaves the map however the run ends (a panic too), after its answer went out.
                    let _flight = Flight(key2.clone());
                    let r = run_and_record(&app2, &req2, &key2, &file_sha, &content, &meta2, params2, cursor2).await;
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
    let id = format!("ev_{}", uuid::Uuid::new_v4().simple());
    let args = params.as_object().cloned().unwrap_or_default();
    let r = run(app, &req.workspace_id, content, meta, &args, &cursor_in, &id).await;
    let at = now_ms();
    let e = Evaluation {
        id,
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
        log: r.log,
        calls: r.calls,
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
            let mut next = prev.next(row.outcome, row.error.as_ref(), row.created_at);
            if let Some(c) = row.error.as_ref().and_then(|e| e.call).and_then(|i| row.calls.get(i)) {
                next.last_call = Some(format!("{} {} · {}", c.tool, c.call, c.outcome));
            }
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
    /// The call of the call log the last error points at (`github GET /repos/… · HTTP 404`).
    #[serde(default)]
    pub last_call: Option<String>,
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
        Streak { workspace_id: ws.into(), oracle: oracle.into(), count: 0, first_at: None, last_at: None, classes: BTreeMap::new(), last_error: None, last_call: None, last_good_at: None, read_at: None, recovered_at: None, recovered_after: 0 }
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
                s.last_call = None;
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

    fn answer(v: Value) -> RunResult {
        decide(&v.to_string())
    }

    #[test]
    fn decides_the_answer() {
        let r = answer(serde_json::json!({"kind": "verdict", "value": true, "summary": "red", "fingerprint": "abc", "links": ["https://x"], "items": [1], "cursor": "c2", "cursor_set": true}));
        assert_eq!((r.outcome, r.summary.as_str(), r.fingerprint.as_deref(), r.cursor), (Outcome::True, "red", Some("abc"), Some(Value::from("c2"))));
        let r = answer(serde_json::json!({"kind": "verdict", "value": false, "summary": "", "fingerprint": null, "links": null, "items": null, "cursor": null, "cursor_set": false}));
        assert_eq!((r.outcome, r.cursor, r.links.len()), (Outcome::False, None, 0));
        // An explicit cursor=None resets it.
        assert_eq!(answer(serde_json::json!({"kind": "verdict", "value": false, "summary": "", "cursor": null, "cursor_set": true})).cursor, Some(Value::Null));
        for bad in [
            serde_json::json!({"kind": "contract", "message": "check returned int"}),
            serde_json::json!({"kind": "verdict", "value": "yes", "summary": ""}),
            serde_json::json!({"kind": "verdict", "value": true, "summary": 1}),
            serde_json::json!({"kind": "verdict", "value": true, "summary": "", "links": [1]}),
            serde_json::json!({"kind": "verdict", "value": true, "summary": "", "cursor": 5, "cursor_set": true}),
            serde_json::json!({"kind": "verdict", "value": true, "summary": "", "cursor": "x".repeat(MAX_CURSOR + 1), "cursor_set": true}),
            serde_json::json!({"kind": "verdict", "value": true, "summary": "x".repeat(MAX_EVIDENCE)}),
        ] {
            assert_eq!(answer(bad.clone()).error.unwrap().class, ErrorClass::Contract, "{bad}");
        }
    }

    /// T14: an error a tool raised keeps its class even when the code catches and re-raises it; other exceptions are
    /// script errors with their line; Monty's limits are limit or timeout.
    #[test]
    fn classifies_exceptions() {
        let raised = vec![(ExcType::PermissionError, "tools.github is not in this oracle's header".to_string(), ErrorClass::Access, 0)];
        let e = classify(&MontyException::new(ExcType::PermissionError, Some("tools.github is not in this oracle's header".into())), &raised);
        assert_eq!((e.class, e.call), (ErrorClass::Access, Some(0)));
        assert_eq!(classify(&MontyException::new(ExcType::KeyError, Some("'a'".into())), &raised).class, ErrorClass::Script);
        assert_eq!(classify(&MontyException::new(ExcType::RuntimeError, Some("suspension limit 250 exceeded".into())), &[]).class, ErrorClass::Limit);
        assert_eq!(classify(&MontyException::new(ExcType::TimeoutError, Some("sleep limit exceeded: 5s > 0ns".into())), &[]).class, ErrorClass::Limit);
        assert_eq!(classify(&MontyException::new(ExcType::MemoryError, Some("memory limit exceeded".into())), &[]).class, ErrorClass::Limit);
        assert_eq!(classify(&MontyException::new(ExcType::TimeoutError, Some("feed time limit exceeded: 10s > 10s".into())), &[]).class, ErrorClass::Timeout);
        // The code's own TimeoutError stays a script error.
        assert_eq!(classify(&MontyException::new(ExcType::TimeoutError, Some("mine".into())), &[]).class, ErrorClass::Script);
    }

    #[test]
    fn cursor_arguments() {
        assert_eq!(cursor_arg(&None), Value::Null);
        assert_eq!(cursor_arg(&Some(Value::from("1.5"))), Value::from("1.5"));
        assert_eq!(cursor_arg(&Some(serde_json::json!({"v": 2}))), Value::from("{\"v\":2}"));
    }

    #[test]
    fn secrets_never_reach_the_result() {
        let mut r = answer(serde_json::json!({"kind": "verdict", "value": true, "summary": "token ghp_secret123", "items": [{"t": "ghp_secret123"}]}));
        r.log = "Authorization: Bearer ghp_secret123".into();
        let r = redact(r, &["ghp_secret123".into(), "short".into()]);
        assert_eq!((r.summary.as_str(), r.log.as_str(), r.items[0]["t"].as_str()), ("token [redacted]", "Authorization: Bearer [redacted]", Some("[redacted]")));
    }

    #[test]
    fn failure_streaks() {
        let http = EvalError { class: ErrorClass::Http, message: "401".into(), call: None };
        let s = Streak::new("w", "o").next(Outcome::Failed, Some(&http), 10).next(Outcome::Failed, Some(&http), 20);
        assert_eq!((s.count, s.first_at, s.last_at, s.open()), (2, Some(10), Some(20), true));
        assert_eq!(s.dominant(), Some(ErrorClass::Http));
        let read = Streak { read_at: Some(25), ..s.clone() };
        assert!(!read.open());
        // The next failure after Mark read starts a new streak.
        let again = read.next(Outcome::Failed, Some(&EvalError { class: ErrorClass::Timeout, message: "t".into(), call: None }), 30);
        assert_eq!((again.count, again.first_at, again.open(), again.dominant()), (1, Some(30), true, Some(ErrorClass::Timeout)));
        let ok = s.next(Outcome::False, None, 40);
        assert_eq!((ok.count, ok.recovered_at, ok.recovered_after, ok.last_good_at, ok.open()), (0, Some(40), 2, Some(40), false));
        // An older evaluation's env class reads as script.
        assert_eq!(serde_json::from_str::<ErrorClass>("\"env\"").unwrap(), ErrorClass::Script);
    }
}

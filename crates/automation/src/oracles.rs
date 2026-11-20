//! Oracles: small Python files that answer one question with True, False or Failed (`oracle_eval` runs them in
//! Monty on Home).
//!
//! Each workspace keeps its oracles as `oracles/<slug>.py` in its own store (`plain::oracle_store`, a plain store
//! that only Home writes, through `plain::write_texts`), so they get Versions, diff and Restore like a bot's files.
//! The file starts with a `# /// oracle` block (TOML: title, description, tools, min_interval, timeout); the
//! parameters are the arguments of `def check(...)`. Every save, draft and card is pre-checked
//! (`oracle_check::precheck`: header, Ruff fix and format, signature, Monty parse, type check) and stored formatted.
//! A stored file that no longer passes (an older format, a Restore) stays stored and shows as broken, with its
//! problems: it is never evaluated until it is saved again. The table `oracle` keeps who created and last changed
//! each one and whether it is paused.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result, bail, ensure};
use launchpad_core::App;
use launchpad_core::kb::Author;
use launchpad_core::plain;
use launchpad_core::util::now_ms;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use ts_rs::TS;

use crate::oracle_check;

/// Folder of the oracle files in the store.
pub const DIR: &str = "oracles/";
/// Largest oracle file.
pub const MAX_FILE: usize = 256 * 1024;
/// Default and largest wall time of one evaluation, tool calls included.
pub const DEFAULT_TIMEOUT_MS: i64 = 10_000;
pub const MAX_TIMEOUT_MS: i64 = 60_000;
/// Shortest (and default) time an answer stays fresh.
pub const MIN_INTERVAL_MS: i64 = 60_000;
/// Largest proposed cursor, and evidence (summary, fingerprint, links and items) as JSON.
pub const MAX_CURSOR: usize = 4 * 1024;
pub const MAX_EVIDENCE: usize = 32 * 1024;
/// The two arguments Home gives `check` itself when its signature asks for them.
pub const RESERVED: [&str; 2] = ["cursor", "now"];

/// A tool an oracle may call (`tools = [...]` in its header).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "OracleTool")]
pub enum Tool {
    /// Any public address, never with a credential.
    Http,
    /// GitHub's REST API with the registry tool gh.
    Github,
    /// The Hugging Face Hub with the registry tool hf.
    Huggingface,
    /// Home's read API for the oracle's own workspace, in process.
    Launchpad,
}

impl Tool {
    pub const ALL: [Tool; 4] = [Tool::Http, Tool::Github, Tool::Huggingface, Tool::Launchpad];

    pub fn name(self) -> &'static str {
        match self {
            Tool::Http => "http",
            Tool::Github => "github",
            Tool::Huggingface => "huggingface",
            Tool::Launchpad => "launchpad",
        }
    }

    pub fn parse(name: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|t| t.name() == name)
    }

    /// The registry tool whose token it uses (`vault::oracle_registry_tool`); None: it needs none.
    pub fn registry(self) -> Option<&'static str> {
        launchpad_core::vault::oracle_registry_tool(self.name())
    }
}

/// The type of a parameter: the annotation of its argument.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "OracleParamKind")]
pub enum ParamKind {
    Str,
    Int,
    Float,
    Bool,
}

impl ParamKind {
    pub fn name(self) -> &'static str {
        match self {
            ParamKind::Str => "str",
            ParamKind::Int => "int",
            ParamKind::Float => "float",
            ParamKind::Bool => "bool",
        }
    }
}

/// One parameter: an argument of `check`.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[ts(rename = "OracleParam")]
pub struct ParamSpec {
    pub name: String,
    pub kind: ParamKind,
    /// `T | None`: None is a value.
    pub optional: bool,
    /// No default: callers must pass it.
    pub required: bool,
    /// The default (None when required; `Some(null)`: defaults to None).
    #[ts(type = "unknown")]
    pub default: Option<Value>,
}

/// The parsed definition of an oracle file: its header and the signature of `check`.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[ts(rename = "OracleMeta")]
pub struct Meta {
    pub title: String,
    /// The question in a sentence.
    pub description: Option<String>,
    pub tools: Vec<Tool>,
    #[ts(type = "number")]
    pub min_interval_ms: i64,
    #[ts(type = "number")]
    pub timeout_ms: i64,
    pub params: Vec<ParamSpec>,
    /// `check` takes `cursor: str | None`.
    pub cursor: bool,
    /// `check` takes `now: datetime`.
    pub now: bool,
}

/// A problem of an oracle file, at its line: a pre-check error (it refuses the file) or a type warning.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[ts(rename = "OracleProblem")]
pub struct Problem {
    pub line: Option<u32>,
    pub column: Option<u32>,
    /// The Ruff or type checker rule (`F821`, `invalid-argument-type`), or the step (`header`, `signature`, `monty`).
    pub rule: Option<String>,
    pub message: String,
}

impl Problem {
    pub fn new(rule: &str, line: Option<u32>, message: impl Into<String>) -> Problem {
        Problem { line, column: None, rule: Some(rule.to_string()), message: message.into() }
    }

    /// `oracle.py:14:9: F821 Undefined name `tool``.
    pub fn text(&self) -> String {
        let mut s = String::from("oracle.py");
        if let Some(l) = self.line {
            s.push_str(&format!(":{l}"));
            if let Some(c) = self.column {
                s.push_str(&format!(":{c}"));
            }
        }
        s.push_str(": ");
        if let Some(r) = &self.rule {
            s.push_str(r);
            s.push(' ');
        }
        s.push_str(&self.message);
        s
    }
}

/// Problems as one text, one per line.
pub fn problems_text(problems: &[Problem]) -> String {
    problems.iter().map(Problem::text).collect::<Vec<_>>().join("\n")
}

/// An oracle of a workspace.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Oracle {
    pub workspace_id: String,
    pub slug: String,
    /// Content hash of the current file.
    pub sha256: String,
    /// None when the header or the signature does not parse.
    pub meta: Option<Meta>,
    /// Why the stored file is broken (it does not pass the pre-check): it is not evaluated until it is saved again.
    pub problems: Vec<Problem>,
    pub created_by: Option<Author>,
    #[ts(type = "number | null")]
    pub created_at: Option<i64>,
    pub changed_by: Option<Author>,
    #[ts(type = "number | null")]
    pub changed_at: Option<i64>,
    pub paused: bool,
}

impl Oracle {
    pub fn broken(&self) -> bool {
        !self.problems.is_empty()
    }

    /// Why it is broken, in one line (the first problem).
    pub fn broken_reason(&self) -> Option<String> {
        self.problems.first().map(Problem::text)
    }
}

/// An oracle with its file.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct OracleFile {
    pub oracle: Oracle,
    pub content: String,
}

// ---------------------------------------------------------------- parsing (pure)

/// `[a-z0-9]`, then up to 62 of `[a-z0-9_-]`.
pub fn valid_slug(slug: &str) -> bool {
    let b = slug.as_bytes();
    !b.is_empty() && b.len() <= 63 && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit()) && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-' || *c == b'_')
}

/// The store path of an oracle.
pub fn path_of(slug: &str) -> String {
    format!("{DIR}{slug}.py")
}

/// The slug of a store path (`oracles/<slug>.py`).
pub fn slug_of(path: &str) -> Option<&str> {
    path.strip_prefix(DIR)?.strip_suffix(".py").filter(|s| valid_slug(s))
}

/// The header block of type `kind` (`# /// oracle`): its TOML and the line (1-based) of its opening line. PEP 723
/// rules: a line `# /// <type>` opens it, every following line is `#` or `# <text>`, and the last `# ///` of those
/// lines closes it; two blocks of one type are an error.
pub fn block(src: &str, kind: &str) -> std::result::Result<Option<(String, u32)>, Problem> {
    let lines: Vec<&str> = src.lines().collect();
    let open = format!("# /// {kind}");
    let mut found: Option<(String, u32)> = None;
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim_end() != open {
            i += 1;
            continue;
        }
        if found.is_some() {
            return Err(Problem::new("header", Some(i as u32 + 1), format!("the file has two `{open}` blocks")));
        }
        let mut j = i + 1;
        let mut end = None;
        while j < lines.len() && (lines[j] == "#" || lines[j].starts_with("# ")) {
            if lines[j].trim_end() == "# ///" {
                end = Some(j);
            }
            j += 1;
        }
        let Some(end) = end else { return Err(Problem::new("header", Some(i as u32 + 1), format!("the `{open}` block is not closed with a `# ///` line"))) };
        let body: Vec<&str> = lines[i + 1..end].iter().map(|l| l.strip_prefix("# ").unwrap_or(l.strip_prefix('#').unwrap_or(l))).collect();
        found = Some((body.join("\n") + "\n", i as u32 + 1));
        i = end + 1;
    }
    Ok(found)
}

/// `90s`, `2m`, `1h`, `1d`, `500ms`, or an integer of seconds.
pub fn parse_duration(v: &toml::Value) -> Result<i64> {
    if let Some(n) = v.as_integer() {
        ensure!(n >= 0, "a duration is not negative");
        return n.checked_mul(1000).context("this duration is too long");
    }
    let s = v.as_str().context("a duration is a string like \"2m\" or a number of seconds")?.trim();
    let split = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (n, unit) = s.split_at(split);
    let n: i64 = n.parse().with_context(|| format!("unreadable duration {s:?}: use 10s, 2m, 1h or 1d"))?;
    let ms = match unit.trim() {
        "ms" => 1,
        "s" | "" => 1000,
        "m" | "min" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        u => bail!("unknown duration unit {u:?} in {s:?}: use ms, s, m, h or d"),
    };
    n.checked_mul(ms).with_context(|| format!("duration {s:?} is too long"))
}

/// `parse_duration` of a text (`2m`).
pub fn duration_text(s: &str) -> Result<i64> {
    parse_duration(&toml::Value::String(s.to_string()))
}

/// The header of an oracle file (everything of `Meta` but the parameters).
#[derive(Clone, Debug, PartialEq)]
pub struct Header {
    pub title: String,
    pub description: Option<String>,
    pub tools: Vec<Tool>,
    pub min_interval_ms: i64,
    pub timeout_ms: i64,
}

const KEYS: [&str; 5] = ["title", "description", "tools", "min_interval", "timeout"];

/// Parses the `# /// oracle` block. Files of the older format (`# /// script` with `[tool.launchpad]`) get a
/// message that says what changed.
pub fn header(src: &str) -> std::result::Result<Header, Problem> {
    let Some((toml_text, line)) = block(src, "oracle")? else {
        if let Ok(Some((_, l))) = block(src, "script") {
            return Err(Problem::new(
                "header",
                Some(l),
                "this oracle uses the older format (`# /// script`, `ctx`, `run(check)`): rewrite it with a `# /// oracle` header (title, description, tools, min_interval, timeout) and `def check(<parameters>)`",
            ));
        }
        return Err(Problem::new("header", Some(1), "an oracle starts with a `# /// oracle` block: `# title = \"…\"` and the other keys, then `# ///`"));
    };
    let at = |m: String| Problem::new("header", Some(line), m);
    let doc: toml::Table = toml::from_str(&toml_text).map_err(|e| at(format!("the `# /// oracle` block is not valid TOML: {}", e.message())))?;
    for k in doc.keys() {
        if !KEYS.contains(&k.as_str()) {
            return Err(at(format!("unknown key {k} in the oracle block (title, description, tools, min_interval, timeout)")));
        }
    }
    let title = doc.get("title").and_then(|t| t.as_str()).map(str::trim).filter(|t| !t.is_empty()).ok_or_else(|| at("the oracle block needs a title".into()))?.to_string();
    let description = match doc.get("description") {
        Some(d) => Some(d.as_str().ok_or_else(|| at("description is a string".into()))?.to_string()),
        None => None,
    };
    let mut tools = vec![];
    if let Some(v) = doc.get("tools") {
        let list = v.as_array().ok_or_else(|| at("tools is a list such as [\"github\", \"http\"]".into()))?;
        for t in list {
            let name = t.as_str().ok_or_else(|| at("tools is a list of names".into()))?;
            let tool = Tool::parse(name).ok_or_else(|| at(format!("unknown tool {name:?}: use http, github, huggingface or launchpad")))?;
            if tools.contains(&tool) {
                return Err(at(format!("tools lists {name} twice")));
            }
            tools.push(tool);
        }
    }
    let min_interval_ms = doc.get("min_interval").map(parse_duration).transpose().map_err(|e| at(format!("min_interval: {e:#}")))?.unwrap_or(MIN_INTERVAL_MS);
    if min_interval_ms < MIN_INTERVAL_MS {
        return Err(at("min_interval is at least 1m".into()));
    }
    let timeout_ms = doc.get("timeout").map(parse_duration).transpose().map_err(|e| at(format!("timeout: {e:#}")))?.unwrap_or(DEFAULT_TIMEOUT_MS);
    if !(500..=MAX_TIMEOUT_MS).contains(&timeout_ms) {
        return Err(at("timeout is between 500ms and 60s".into()));
    }
    Ok(Header { title, description, tools, min_interval_ms, timeout_ms })
}

/// Pure: the parsed definition of a file (header, then the signature of `check`).
pub fn meta_of(src: &str) -> std::result::Result<Meta, Problem> {
    let h = header(src)?;
    let sig = oracle_check::signature(src)?;
    Ok(Meta { title: h.title, description: h.description, tools: h.tools, min_interval_ms: h.min_interval_ms, timeout_ms: h.timeout_ms, params: sig.params, cursor: sig.cursor, now: sig.now })
}

/// A value of a parameter's type, from a typed JSON value or from a string (`--param k=v`).
pub fn convert(p: &ParamSpec, v: &Value) -> std::result::Result<Value, String> {
    if v.is_null() {
        return if p.optional { Ok(Value::Null) } else { Err(format!("{} is a {}, not None", p.name, p.kind.name())) };
    }
    let s = v.as_str();
    let bad = |what: &str| Err(format!("{} is a {} ({what}), not {}", p.name, p.kind.name(), v));
    match p.kind {
        ParamKind::Str => match v {
            Value::String(_) => Ok(v.clone()),
            _ => bad("text"),
        },
        ParamKind::Bool => match (v, s.map(|x| x.trim().to_ascii_lowercase())) {
            (Value::Bool(_), _) => Ok(v.clone()),
            (_, Some(x)) if x == "true" => Ok(Value::Bool(true)),
            (_, Some(x)) if x == "false" => Ok(Value::Bool(false)),
            _ => bad("true or false"),
        },
        ParamKind::Int => match (v, s) {
            (Value::Number(n), _) if n.is_i64() => Ok(v.clone()),
            (_, Some(x)) => x.trim().parse::<i64>().map(Value::from).or_else(|_| bad("a whole number")),
            _ => bad("a whole number"),
        },
        ParamKind::Float => match (v.as_f64(), s) {
            (Some(n), _) if v.is_number() && n.is_finite() => Ok(Value::from(n)),
            (_, Some(x)) => x.trim().parse::<f64>().ok().filter(|f| f.is_finite()).map(Value::from).map_or_else(|| bad("a number"), Ok),
            _ => bad("a number"),
        },
    }
}

/// `k=v` arguments (`--param`), as strings for `validate_params`.
pub fn parse_param_args(args: &[String]) -> Result<Map<String, Value>> {
    let mut out = Map::new();
    for a in args {
        let (k, v) = a.split_once('=').with_context(|| format!("--param {a:?}: use name=value"))?;
        ensure!(out.insert(k.trim().to_string(), Value::String(v.to_string())).is_none(), "--param {k} is given twice");
    }
    Ok(out)
}

/// The arguments an evaluation passes: each given one checked against its annotation, the defaults of the others.
/// Unknown, reserved (`cursor`, `now`) and missing required arguments are refused, naming the argument.
pub fn validate_params(specs: &[ParamSpec], given: &Map<String, Value>) -> Result<Map<String, Value>> {
    for k in given.keys() {
        if RESERVED.contains(&k.as_str()) {
            bail!("{k} is not a parameter: Home gives it to check itself ({})", if k == "cursor" { "use --cursor <name> or --since <value>" } else { "the evaluation's start" });
        }
        ensure!(specs.iter().any(|p| &p.name == k), "unknown parameter {k} (check takes {})", if specs.is_empty() { "none".to_string() } else { specs.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ") });
    }
    let mut out = Map::new();
    for p in specs {
        let v = match given.get(&p.name) {
            Some(v) => convert(p, v).map_err(anyhow::Error::msg)?,
            None => p.default.clone().with_context(|| format!("parameter {} is required ({})", p.name, p.kind.name()))?,
        };
        out.insert(p.name.clone(), v);
    }
    Ok(out)
}

/// The tools a new definition adds and removes.
pub fn tools_diff(old: &[Tool], new: &[Tool]) -> (Vec<Tool>, Vec<Tool>) {
    (new.iter().filter(|t| !old.contains(t)).copied().collect(), old.iter().filter(|t| !new.contains(t)).copied().collect())
}

/// A credentialed tool together with http: the oracle can send what it reads to any public address.
pub fn can_leak(tools: &[Tool]) -> bool {
    tools.contains(&Tool::Http) && tools.iter().any(|t| t.registry().is_some())
}

/// The credentialed tools (github, huggingface): the ones an agent's save or draft must hold itself.
pub fn credentialed(tools: &[Tool]) -> Vec<Tool> {
    tools.iter().filter(|t| t.registry().is_some()).copied().collect()
}

// ---------------------------------------------------------------- the store (Home)

fn ensure_home(app: &App) -> Result<()> {
    ensure!(launchpad_core::home_git::here(app), "oracles are stored by Home");
    Ok(())
}

#[derive(Default)]
struct Row {
    created_by: Option<Author>,
    created_at: Option<i64>,
    changed_by: Option<Author>,
    changed_at: Option<i64>,
    paused: bool,
}

async fn rows(app: &App, ws: &str) -> Result<BTreeMap<String, Row>> {
    let ws = ws.to_string();
    app.state_db
        .call(move |c| {
            let mut st = c.prepare("SELECT slug, created_by, created_at, changed_by, changed_at, paused FROM oracle WHERE workspace_id = ?1")?;
            let rows = st
                .query_map([ws], |r| {
                    let author = |s: String| serde_json::from_str::<Author>(&s).ok();
                    Ok((r.get::<_, String>(0)?, Row { created_by: author(r.get(1)?), created_at: r.get(2)?, changed_by: author(r.get(3)?), changed_at: r.get(4)?, paused: r.get::<_, i64>(5)? != 0 }))
                })?
                .collect::<rusqlite::Result<BTreeMap<_, _>>>()?;
            Ok(rows)
        })
        .await
}

async fn text(app: &App, sha: &str) -> String {
    plain::read_blob(app, sha).await.map(|d| String::from_utf8_lossy(&d).into_owned()).unwrap_or_default()
}

/// What a stored file is: its definition and its problems, decided from the file alone and kept by file hash.
#[derive(Clone)]
struct Inspection {
    meta: Option<Meta>,
    problems: Vec<Problem>,
}

fn inspections() -> &'static Mutex<HashMap<String, Inspection>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Inspection>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// A stored file's definition and problems: the in-process checks (header, signature, Monty parse) and Ruff's
/// errors it cannot fix (skipped when Ruff is missing on Home), cached by hash.
async fn inspect(app: &App, sha: &str, content: &str) -> Inspection {
    if let Some(i) = inspections().lock().unwrap_or_else(|p| p.into_inner()).get(sha) {
        return i.clone();
    }
    let mut problems = vec![];
    let meta = match meta_of(content) {
        Ok(m) => Some(m),
        Err(p) => {
            problems.push(p);
            None
        }
    };
    if problems.is_empty() {
        if let Err(p) = oracle_check::monty_parse(content) {
            problems.push(p);
        } else if let Some(lint) = oracle_check::lint_errors(app, content).await {
            problems.extend(lint);
        }
    }
    let i = Inspection { meta, problems };
    let mut cache = inspections().lock().unwrap_or_else(|p| p.into_inner());
    if cache.len() > 1024 {
        cache.clear();
    }
    cache.insert(sha.to_string(), i.clone());
    i
}

async fn record(app: &App, ws: &str, slug: &str, sha: &str, content: &str, row: Row) -> Oracle {
    let i = inspect(app, sha, content).await;
    Oracle { workspace_id: ws.into(), slug: slug.into(), sha256: sha.into(), meta: i.meta, problems: i.problems, created_by: row.created_by, created_at: row.created_at, changed_by: row.changed_by, changed_at: row.changed_at, paused: row.paused }
}

/// The oracles of a workspace, by slug.
pub async fn list(app: &App, ws: &str) -> Result<Vec<Oracle>> {
    ensure_home(app)?;
    let tree = plain::tree(app, &plain::oracle_store(ws)).await?;
    let mut meta = rows(app, ws).await?;
    let mut out = vec![];
    for f in &tree.files {
        let Some(slug) = slug_of(&f.path) else { continue };
        let content = text(app, &f.sha256).await;
        out.push(record(app, ws, slug, &f.sha256, &content, meta.remove(slug).unwrap_or_default()).await);
    }
    Ok(out)
}

/// One oracle with its file.
pub async fn get(app: &App, ws: &str, slug: &str) -> Result<OracleFile> {
    ensure_home(app)?;
    ensure!(valid_slug(slug), "invalid oracle name {slug:?}");
    let tree = plain::tree(app, &plain::oracle_store(ws)).await?;
    let path = path_of(slug);
    let f = tree.files.iter().find(|f| f.path == path).with_context(|| format!("no oracle {slug} in this workspace"))?;
    let content = text(app, &f.sha256).await;
    let row = rows(app, ws).await?.remove(slug).unwrap_or_default();
    Ok(OracleFile { oracle: record(app, ws, slug, &f.sha256, &content, row).await, content })
}

async fn touch(app: &App, ws: &str, slug: &str, author: &Author) -> Result<()> {
    let (ws, slug, json, at) = (ws.to_string(), slug.to_string(), serde_json::to_string(author)?, now_ms());
    app.state_db
        .call(move |c| {
            c.execute(
                "INSERT INTO oracle (workspace_id, slug, created_by, created_at, changed_by, changed_at) VALUES (?1, ?2, ?3, ?4, ?3, ?4)
                 ON CONFLICT (workspace_id, slug) DO UPDATE SET changed_by = ?3, changed_at = ?4",
                params![ws, slug, json, at],
            )?;
            Ok(())
        })
        .await
}

/// What a save answers: the oracle, the type warnings of the saved file, the tools it adds and removes, and the
/// credentialed tools not granted to the workspace or without a token (it would fail with class access).
#[derive(Clone, Debug, Serialize, TS)]
#[ts(rename = "OracleSaved")]
pub struct Saved {
    pub oracle: Oracle,
    pub warnings: Vec<Problem>,
    pub tools_added: Vec<Tool>,
    pub tools_removed: Vec<Tool>,
    pub tools_missing: Vec<String>,
}

/// The tools of the oracle stored now (none for a new or broken one).
pub async fn stored_tools(app: &App, ws: &str, slug: &str) -> Vec<Tool> {
    get(app, ws, slug).await.ok().and_then(|f| f.oracle.meta).map(|m| m.tools).unwrap_or_default()
}

/// Why each credentialed tool of this list would fail in the workspace (not granted, no token).
pub async fn missing_grants(app: &App, ws: &str, tools: &[Tool]) -> Vec<String> {
    let mut out = vec![];
    for t in tools {
        if let Some(id) = t.registry()
            && let Err(why) = launchpad_core::vault::oracle_token(app, ws, id).await
        {
            out.push(format!("{}: {why}", t.name()));
        }
    }
    out
}

/// Creates or changes an oracle: pre-checks the file (`oracle_check::precheck`) and stores its formatted text.
/// Who may (the owner; an admin bot for the credentialed tools its session holds; a card otherwise) is the caller's
/// rule. Err with `oracle_check::Refused` when the pre-check refuses the file.
pub async fn save(app: &App, ws: &str, slug: &str, content: &str, author: Author) -> Result<Saved> {
    ensure_home(app)?;
    ensure!(valid_slug(slug), "an oracle name uses lowercase letters, digits, - and _ (at most 63)");
    launchpad_core::workspace::get(app, ws).await?;
    let checked = oracle_check::precheck(app, content).await?;
    let old = stored_tools(app, ws, slug).await;
    if plain::write_texts(app, &plain::oracle_store(ws), vec![(path_of(slug), Some(checked.content.clone()))], author.clone()).await? {
        touch(app, ws, slug, &author).await?;
        changed(app, ws, slug);
    }
    let (tools_added, tools_removed) = tools_diff(&old, &checked.meta.tools);
    let tools_missing = missing_grants(app, ws, &checked.meta.tools).await;
    Ok(Saved { oracle: get(app, ws, slug).await?.oracle, warnings: checked.warnings, tools_added, tools_removed, tools_missing })
}

/// Deletes an oracle (its file stays restorable 30 days); its evaluations, cursors and failure streak go with it.
/// Checking what uses it is the caller's.
pub async fn delete(app: &App, ws: &str, slug: &str, author: Author) -> Result<()> {
    get(app, ws, slug).await?;
    plain::write_texts(app, &plain::oracle_store(ws), vec![(path_of(slug), None)], author).await?;
    let (ws2, slug2) = (ws.to_string(), slug.to_string());
    app.state_db
        .call(move |c| {
            let tx = c.transaction()?;
            for t in ["oracle_eval", "oracle_cursor", "oracle_streak"] {
                tx.execute(&format!("DELETE FROM {t} WHERE workspace_id = ?1 AND oracle = ?2"), params![ws2, slug2])?;
            }
            tx.execute("DELETE FROM oracle WHERE workspace_id = ?1 AND slug = ?2", params![ws2, slug2])?;
            tx.commit()?;
            Ok(())
        })
        .await?;
    changed(app, ws, slug);
    Ok(())
}

/// Pauses an oracle (it is not evaluated: callers get an error, gates a quiet check) or resumes it.
pub async fn set_paused(app: &App, ws: &str, slug: &str, paused: bool) -> Result<Oracle> {
    get(app, ws, slug).await?;
    let (ws2, slug2) = (ws.to_string(), slug.to_string());
    let (json, at) = (serde_json::to_string(&Author::owner(None))?, now_ms());
    app.state_db
        .call(move |c| {
            c.execute(
                "INSERT INTO oracle (workspace_id, slug, created_by, created_at, changed_by, changed_at, paused) VALUES (?1, ?2, ?3, ?4, ?3, ?4, ?5)
                 ON CONFLICT (workspace_id, slug) DO UPDATE SET paused = ?5",
                params![ws2, slug2, json, at, paused as i64],
            )?;
            Ok(())
        })
        .await?;
    changed(app, ws, slug);
    Ok(get(app, ws, slug).await?.oracle)
}

/// Changes of the workspace's oracle files in the last 30 days, newest first (one oracle's with `slug`).
pub async fn history(app: &App, ws: &str, slug: Option<&str>) -> Result<Vec<plain::Change>> {
    ensure_home(app)?;
    let all = plain::history(app, &plain::oracle_store(ws)).await?;
    Ok(match slug {
        Some(s) => {
            let path = path_of(s);
            all.into_iter().filter(|c| c.path == path).collect()
        }
        None => all.into_iter().filter(|c| slug_of(&c.path).is_some()).collect(),
    })
}

async fn change(app: &App, ws: &str, id: i64) -> Result<plain::Change> {
    history(app, ws, None).await?.into_iter().find(|c| c.id == id).context("this change is no longer kept")
}

async fn blob(app: &App, sha: Option<&str>) -> String {
    match sha {
        Some(h) => text(app, h).await,
        None => String::new(),
    }
}

/// The unified diff of one change.
pub async fn diff(app: &App, ws: &str, id: i64) -> Result<String> {
    let c = change(app, ws, id).await?;
    Ok(launchpad_core::kb::unified(&blob(app, c.sha256.as_deref()).await, &blob(app, c.new_sha256.as_deref()).await))
}

/// The file's content before a change (None: the change created it).
pub async fn before(app: &App, ws: &str, id: i64) -> Result<(String, Option<String>)> {
    let c = change(app, ws, id).await?;
    let slug = slug_of(&c.path).context("not an oracle file")?.to_string();
    Ok((slug, match c.sha256 { Some(h) => Some(text(app, &h).await), None => None }))
}

/// Restore: puts back the file as it was before a change (removes it when the change created it), as a new change
/// by `author`. The file comes back as it was, even when it no longer passes the pre-check: it then shows as broken.
pub async fn restore(app: &App, ws: &str, id: i64, author: Author) -> Result<()> {
    let (slug, content) = before(app, ws, id).await?;
    let gone = content.is_none();
    plain::write_texts(app, &plain::oracle_store(ws), vec![(path_of(&slug), content)], author.clone()).await?;
    if !gone {
        touch(app, ws, &slug, &author).await?;
    }
    changed(app, ws, &slug);
    Ok(())
}

fn changed(app: &App, ws: &str, slug: &str) {
    app.bus.publish("oracles", "changed", serde_json::json!({"workspace_id": ws, "slug": slug}));
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "# /// oracle\n# title = \"CI red\"\n# description = \"Is CI red?\"\n# tools = [\"github\", \"http\"]\n# min_interval = \"2m\"\n# timeout = \"10s\"\n# ///\nfrom launchpad_oracle import tools, verdict\n\n\ndef check(cursor: str | None, repo: str, branch: str = \"main\", limit: int = 20, hours: float = 2, drafts: bool = False, label: str | None = None):\n    return True\n";

    #[test]
    fn parses_the_header() {
        let h = header(FILE).unwrap();
        assert_eq!((h.title.as_str(), h.description.as_deref(), h.tools.clone()), ("CI red", Some("Is CI red?"), vec![Tool::Github, Tool::Http]));
        assert_eq!((h.min_interval_ms, h.timeout_ms), (120_000, 10_000));
        let d = header("# /// oracle\n# title = \"x\"\n# ///\n").unwrap();
        assert_eq!((d.tools.len(), d.min_interval_ms, d.timeout_ms), (0, MIN_INTERVAL_MS, DEFAULT_TIMEOUT_MS));
    }

    #[test]
    fn header_errors_name_their_line() {
        let e = header("import os\n# /// oracle\n# title = \"x\"\n# acess = 1\n# ///\n").unwrap_err();
        assert_eq!((e.line, e.rule.as_deref()), (Some(2), Some("header")));
        assert!(e.message.contains("unknown key acess"));
        for bad in ["# tools = [\"gh\"]", "# timeout = \"2m\"", "# min_interval = \"30s\"", "# tools = [\"http\", \"http\"]", "# params = {}", "# dependencies = []"] {
            assert!(header(&format!("# /// oracle\n# title = \"x\"\n{bad}\n# ///\n")).is_err(), "{bad}");
        }
        assert!(header("# /// oracle\n# ///\n").unwrap_err().message.contains("title"));
        assert!(header("# /// oracle\n# title = \"x\"\nx = 1\n").unwrap_err().message.contains("not closed"));
        assert!(header("x = 1\n").unwrap_err().message.contains("starts with"));
        let old = header("# /// script\n# [tool.launchpad]\n# title = \"x\"\n# ///\nfrom launchpad_oracle import run\n").unwrap_err();
        assert!(old.message.contains("older format"), "{}", old.message);
    }

    #[test]
    fn block_edge_cases() {
        assert_eq!(block("import os\n", "oracle").unwrap(), None);
        assert_eq!(block("# /// oracle\n# a = 1\n# ///\n# b = 2\n# ///\nx = 1\n", "oracle").unwrap().unwrap().0, "a = 1\n///\nb = 2\n");
        assert_eq!(block("# /// oracle\r\n# a = 1\r\n#\r\n# ///\r\n", "oracle").unwrap().unwrap().0, "a = 1\n\n");
        assert!(block("# /// oracle\n# ///\nx = 1\n# /// oracle\n# ///\n", "oracle").is_err());
    }

    #[test]
    fn params_follow_the_signature() {
        let m = meta_of(FILE).unwrap();
        assert!(m.cursor && !m.now);
        let given = parse_param_args(&["repo=acme/storefront".into(), "drafts=true".into(), "hours=1.5".into(), "limit=50".into()]).unwrap();
        let v = validate_params(&m.params, &given).unwrap();
        assert_eq!(Value::Object(v), serde_json::json!({"repo": "acme/storefront", "branch": "main", "limit": 50, "hours": 1.5, "drafts": true, "label": null}));
        // T24: reserved names, a mistyped value, an unknown name and a missing required argument are refused, naming it.
        let err = |args: &[&str]| validate_params(&m.params, &parse_param_args(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()).unwrap_err().to_string();
        assert!(err(&["repo=a/b", "cursor=x"]).contains("cursor is not a parameter"));
        assert!(err(&["repo=a/b", "limit=abc"]).contains("limit is a int"));
        assert!(err(&["repo=a/b", "nope=1"]).contains("unknown parameter nope"));
        assert!(err(&[]).contains("repo is required"));
        // Typed values (gates, the window) keep their type; an int is a float.
        let typed: Map<String, Value> = serde_json::from_value(serde_json::json!({"repo": "a/b", "hours": 3, "label": null})).unwrap();
        let v = validate_params(&m.params, &typed).unwrap();
        assert_eq!((v["hours"].as_f64(), v["label"].clone()), (Some(3.0), Value::Null));
        let not_str: Map<String, Value> = serde_json::from_value(serde_json::json!({"repo": 5})).unwrap();
        assert!(validate_params(&m.params, &not_str).is_err());
        assert!(parse_param_args(&["a=1".into(), "a=2".into()]).is_err());
    }

    #[test]
    fn tools_diff_and_leaks() {
        assert_eq!(tools_diff(&[Tool::Github], &[Tool::Github, Tool::Huggingface]), (vec![Tool::Huggingface], vec![]));
        assert_eq!(tools_diff(&[Tool::Github, Tool::Http], &[Tool::Http]), (vec![], vec![Tool::Github]));
        assert!(can_leak(&[Tool::Http, Tool::Huggingface]) && !can_leak(&[Tool::Http, Tool::Launchpad]) && !can_leak(&[Tool::Github]));
        assert_eq!(credentialed(&[Tool::Http, Tool::Launchpad, Tool::Huggingface]), vec![Tool::Huggingface]);
    }

    #[test]
    fn problems_read_like_ruff() {
        let p = Problem { line: Some(14), column: Some(9), rule: Some("F821".into()), message: "Undefined name `tool`".into() };
        assert_eq!(p.text(), "oracle.py:14:9: F821 Undefined name `tool`");
    }
}

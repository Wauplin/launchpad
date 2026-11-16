//! Oracles: small Python files that answer one question with True, False or Failed (`oracle_eval` runs them).
//!
//! Each workspace keeps its oracles as `oracles/<slug>.py` in its own store (`plain::oracle_store`, a plain store
//! that only Home writes, through `plain::write_texts`), so they get Versions, diff and Restore like a bot's files.
//! The file starts with a PEP 723 block (`# /// script`) holding `requires-python`, `dependencies` and a
//! `[tool.launchpad]` table: title, typed parameters, limits and access (default none). The table `oracle` keeps
//! who created and last changed each one and whether it is paused.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail, ensure};
use launchpad_core::App;
use launchpad_core::kb::Author;
use launchpad_core::plain;
use launchpad_core::util::now_ms;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use ts_rs::TS;

/// Folder of the oracle files in the store.
pub const DIR: &str = "oracles/";
/// Largest oracle file.
pub const MAX_FILE: usize = 256 * 1024;
/// Default and largest wall time of one evaluation.
pub const DEFAULT_TIMEOUT_MS: i64 = 10_000;
pub const MAX_TIMEOUT_MS: i64 = 60_000;
/// Shortest (and default) time between two evaluations with the same file, parameters and cursor.
pub const MIN_INTERVAL_MS: i64 = 60_000;
/// Largest proposed cursor and evidence (summary, fingerprint, links and items) as JSON.
pub const MAX_CURSOR: usize = 4 * 1024;
pub const MAX_EVIDENCE: usize = 32 * 1024;

/// The Launchpad state an oracle may read.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "OracleLaunchpadAccess")]
pub enum LaunchpadRead {
    #[default]
    None,
    /// Home's read API for its own workspace.
    Read,
}

/// What an oracle can reach: the real tokens of its tools (each also allows the tool's hosts) and the other hosts
/// `ctx.http` may call (`*` = all).
#[derive(Clone, Debug, Default, Serialize, Deserialize, TS, PartialEq)]
#[ts(rename = "OracleAccess")]
pub struct Access {
    pub tools: Vec<String>,
    pub hosts: Vec<String>,
    pub launchpad: LaunchpadRead,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "OracleParamKind")]
pub enum ParamKind {
    String,
    /// `owner/name` on GitHub.
    Repo,
    Bool,
    Int,
    Number,
}

/// One parameter: `"repo"`, `"string:main"`, `"bool:false"` (type, then an optional default after `:`).
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[ts(rename = "OracleParam")]
pub struct ParamSpec {
    pub name: String,
    pub kind: ParamKind,
    /// None: required.
    #[ts(type = "unknown")]
    pub default: Option<Value>,
}

/// The parsed metadata of an oracle file.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[ts(rename = "OracleMeta")]
pub struct Meta {
    pub title: String,
    /// The question in a sentence (`description`).
    pub description: Option<String>,
    pub requires_python: Option<String>,
    pub dependencies: Vec<String>,
    pub params: Vec<ParamSpec>,
    #[ts(type = "number")]
    pub min_interval_ms: i64,
    #[ts(type = "number")]
    pub timeout_ms: i64,
    pub access: Access,
}

/// An oracle of a workspace.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Oracle {
    pub workspace_id: String,
    pub slug: String,
    /// Content hash of the current file.
    pub sha256: String,
    /// None when the file has an error (`error`): it is not evaluated.
    pub meta: Option<Meta>,
    pub error: Option<String>,
    pub created_by: Option<Author>,
    #[ts(type = "number | null")]
    pub created_at: Option<i64>,
    pub changed_by: Option<Author>,
    #[ts(type = "number | null")]
    pub changed_at: Option<i64>,
    pub paused: bool,
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

/// The TOML of the file's PEP 723 `script` block (None: there is none). The reference rules: a line `# /// script`
/// opens it, every following line is `#` or `# <text>`, and the last `# ///` of those lines closes it; two
/// `script` blocks are an error.
pub fn script_block(src: &str) -> Result<Option<String>> {
    let lines: Vec<&str> = src.lines().collect();
    let mut found: Option<String> = None;
    let mut i = 0;
    while i < lines.len() {
        if lines[i] != "# /// script" {
            i += 1;
            continue;
        }
        ensure!(found.is_none(), "the file has two `# /// script` blocks");
        let mut j = i + 1;
        let mut end = None;
        while j < lines.len() && (lines[j] == "#" || lines[j].starts_with("# ")) {
            if lines[j] == "# ///" {
                end = Some(j);
            }
            j += 1;
        }
        let end = end.context("the `# /// script` block is not closed with a `# ///` line")?;
        let body: Vec<&str> = lines[i + 1..end].iter().map(|l| l.strip_prefix("# ").unwrap_or(l.strip_prefix('#').unwrap_or(l))).collect();
        found = Some(body.join("\n") + "\n");
        i = end + 1;
    }
    Ok(found)
}

/// `90s`, `2m`, `1h`, `1d`, `500ms`, or an integer of seconds.
pub fn parse_duration(v: &toml::Value) -> Result<i64> {
    if let Some(n) = v.as_integer() {
        ensure!(n >= 0, "a duration is not negative");
        return Ok(n * 1000);
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
    Ok(n * ms)
}

/// Parses a parameter spec (`"string:main"`).
pub fn parse_param(name: &str, spec: &str) -> Result<ParamSpec> {
    let ok = name.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c == '_') && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    ensure!(ok, "parameter name {name:?}: use lowercase letters, digits and _");
    let (kind, default) = match spec.split_once(':') {
        Some((k, d)) => (k, Some(d)),
        None => (spec, None),
    };
    let kind = match kind.trim() {
        "string" | "str" => ParamKind::String,
        "repo" => ParamKind::Repo,
        "bool" => ParamKind::Bool,
        "int" => ParamKind::Int,
        "number" | "float" => ParamKind::Number,
        k => bail!("parameter {name}: unknown type {k:?} (string, repo, bool, int or number)"),
    };
    let default = default.map(|d| convert(kind, &Value::String(d.to_string()))).transpose().with_context(|| format!("parameter {name}: default"))?;
    Ok(ParamSpec { name: name.into(), kind, default })
}

/// A value of a parameter's type, from a typed JSON value or from a string (`--param k=v`).
pub fn convert(kind: ParamKind, v: &Value) -> Result<Value> {
    let s = v.as_str();
    Ok(match kind {
        ParamKind::String => match v {
            Value::String(_) => v.clone(),
            Value::Number(n) => Value::String(n.to_string()),
            Value::Bool(b) => Value::String(b.to_string()),
            _ => bail!("expected a string"),
        },
        ParamKind::Repo => {
            let s = s.context("expected owner/name")?.trim();
            let ok = s.split_once('/').is_some_and(|(o, n)| launchpad_core::github::valid_name(o) && launchpad_core::github::valid_name(n));
            ensure!(ok, "expected a GitHub repository owner/name, not {s:?}");
            Value::String(s.to_string())
        }
        ParamKind::Bool => match (v, s.map(|x| x.trim().to_ascii_lowercase())) {
            (Value::Bool(_), _) => v.clone(),
            (_, Some(x)) if ["true", "yes", "1", "on"].contains(&x.as_str()) => Value::Bool(true),
            (_, Some(x)) if ["false", "no", "0", "off"].contains(&x.as_str()) => Value::Bool(false),
            _ => bail!("expected true or false"),
        },
        ParamKind::Int => match (v.as_i64(), s) {
            (Some(n), _) => Value::from(n),
            (None, Some(x)) => Value::from(x.trim().parse::<i64>().with_context(|| format!("expected an integer, not {x:?}"))?),
            _ => bail!("expected an integer"),
        },
        ParamKind::Number => match (v.as_f64(), s) {
            (Some(n), _) => Value::from(n),
            (None, Some(x)) => Value::from(x.trim().parse::<f64>().ok().filter(|f| f.is_finite()).with_context(|| format!("expected a number, not {x:?}"))?),
            _ => bail!("expected a number"),
        },
    })
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

/// The parameters an evaluation gets: each given one converted to its type, the defaults of the others. Unknown
/// and missing required parameters are refused.
pub fn validate_params(specs: &[ParamSpec], given: &Map<String, Value>) -> Result<Map<String, Value>> {
    for k in given.keys() {
        ensure!(specs.iter().any(|p| &p.name == k), "unknown parameter {k} (this oracle takes {})", if specs.is_empty() { "none".to_string() } else { specs.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ") });
    }
    let mut out = Map::new();
    for p in specs {
        let v = match given.get(&p.name) {
            Some(v) => convert(p.kind, v).with_context(|| format!("parameter {}", p.name))?,
            None => p.default.clone().with_context(|| format!("parameter {} is required", p.name))?,
        };
        out.insert(p.name.clone(), v);
    }
    Ok(out)
}

/// A host entry: `*`, a DNS name, or `*.` and a DNS name.
fn valid_host(h: &str) -> bool {
    h == "*" || launchpad_core::vault::dns_name(h.strip_prefix("*.").unwrap_or(h))
}

/// Parses an oracle file's metadata.
pub fn parse(src: &str) -> Result<Meta> {
    let block = script_block(src)?.context("an oracle starts with a PEP 723 block: `# /// script`, its TOML lines, then `# ///`")?;
    let doc: toml::Table = toml::from_str(&block).context("the `# /// script` block is not valid TOML")?;
    for k in doc.keys() {
        ensure!(["requires-python", "dependencies", "tool"].contains(&k.as_str()), "unknown key {k} in the script block");
    }
    let requires_python = match doc.get("requires-python") {
        Some(v) => Some(v.as_str().context("requires-python is a string like \">=3.11\"")?.to_string()),
        None => None,
    };
    let dependencies = match doc.get("dependencies") {
        Some(v) => v.as_array().context("dependencies is a list of strings")?.iter().map(|d| d.as_str().map(str::to_string).context("dependencies is a list of strings")).collect::<Result<Vec<_>>>()?,
        None => vec![],
    };
    let lp = doc.get("tool").and_then(|t| t.get("launchpad")).and_then(|t| t.as_table()).context("the script block needs a [tool.launchpad] table with at least a title")?;
    for k in lp.keys() {
        ensure!(["title", "description", "params", "min_interval", "timeout", "access"].contains(&k.as_str()), "unknown key {k} in [tool.launchpad] (title, description, params, min_interval, timeout, access)");
    }
    let title = lp.get("title").and_then(|t| t.as_str()).map(str::trim).filter(|t| !t.is_empty()).context("[tool.launchpad] needs a title")?.to_string();
    let description = match lp.get("description") {
        Some(d) => Some(d.as_str().context("description is a string")?.to_string()),
        None => None,
    };
    let mut params = vec![];
    if let Some(p) = lp.get("params") {
        for (name, spec) in p.as_table().context("params is a table: name = \"type\" or \"type:default\"")? {
            params.push(parse_param(name, spec.as_str().with_context(|| format!("parameter {name}: a string like \"string:main\""))?)?);
        }
    }
    let min_interval_ms = lp.get("min_interval").map(parse_duration).transpose().context("min_interval")?.unwrap_or(MIN_INTERVAL_MS);
    ensure!(min_interval_ms >= MIN_INTERVAL_MS, "min_interval is at least 1m");
    let timeout_ms = lp.get("timeout").map(parse_duration).transpose().context("timeout")?.unwrap_or(DEFAULT_TIMEOUT_MS);
    ensure!((1000..=MAX_TIMEOUT_MS).contains(&timeout_ms), "timeout is between 1s and 60s");
    let mut access = Access::default();
    if let Some(a) = lp.get("access") {
        let a = a.as_table().context("access is a table: { tools = [...], hosts = [...], launchpad = \"none\" }")?;
        let list = |k: &str| -> Result<Vec<String>> {
            match a.get(k) {
                Some(v) => v.as_array().with_context(|| format!("access.{k} is a list of strings"))?.iter().map(|x| x.as_str().map(|s| s.trim().to_ascii_lowercase()).with_context(|| format!("access.{k} is a list of strings"))).collect(),
                None => Ok(vec![]),
            }
        };
        for k in a.keys() {
            ensure!(["tools", "hosts", "launchpad"].contains(&k.as_str()), "unknown key access.{k} (tools, hosts, launchpad)");
        }
        access.tools = list("tools")?;
        access.hosts = list("hosts")?;
        for h in &access.hosts {
            ensure!(valid_host(h), "access.hosts: {h:?} is not a host name (or * for all hosts)");
        }
        access.launchpad = match a.get("launchpad").map(|v| v.as_str()) {
            None | Some(Some("none")) => LaunchpadRead::None,
            Some(Some("read")) => LaunchpadRead::Read,
            _ => bail!("access.launchpad is \"none\" or \"read\""),
        };
    }
    Ok(Meta { title, description, requires_python, dependencies, params, min_interval_ms, timeout_ms, access })
}

/// What a new definition adds to the access of the old one (tools, hosts, Launchpad read): an agent never applies
/// such a change itself (a card waits for the owner).
pub fn widens(old: Option<&Access>, new: &Access) -> Vec<String> {
    let empty = Access::default();
    let old = old.unwrap_or(&empty);
    let mut out: Vec<String> = new.tools.iter().filter(|t| !old.tools.contains(t)).map(|t| format!("tool {t}")).collect();
    if !old.hosts.iter().any(|h| h == "*") {
        out.extend(new.hosts.iter().filter(|h| !old.hosts.contains(h)).map(|h| if h == "*" { "every host (*)".to_string() } else { format!("host {h}") }));
    }
    if new.launchpad == LaunchpadRead::Read && old.launchpad != LaunchpadRead::Read {
        out.push("Launchpad read".into());
    }
    out
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

fn record(ws: &str, slug: &str, sha: &str, content: &str, row: Row) -> Oracle {
    let (meta, error) = match parse(content) {
        Ok(m) => (Some(m), None),
        Err(e) => (None, Some(format!("{e:#}"))),
    };
    Oracle { workspace_id: ws.into(), slug: slug.into(), sha256: sha.into(), meta, error, created_by: row.created_by, created_at: row.created_at, changed_by: row.changed_by, changed_at: row.changed_at, paused: row.paused }
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
        out.push(record(ws, slug, &f.sha256, &content, meta.remove(slug).unwrap_or_default()));
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
    Ok(OracleFile { oracle: record(ws, slug, &f.sha256, &content, row), content })
}

/// Checks a new oracle file: its metadata parses and its tools are in Home's registry.
pub async fn check(app: &App, content: &str) -> Result<Meta> {
    ensure!(content.len() <= MAX_FILE, "an oracle file is at most 256 KB");
    let meta = parse(content)?;
    let tools = launchpad_core::vault::tools(app).await?;
    for t in &meta.access.tools {
        ensure!(tools.iter().any(|x| &x.id == t), "access.tools: there is no tool {t} in Home's registry (Settings → Credentials and tools)");
    }
    Ok(meta)
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

/// Creates or changes an oracle (checked with `check`). Who may (the owner, an admin bot for code, a card for
/// access) is the caller's rule: see `widens`.
pub async fn save(app: &App, ws: &str, slug: &str, content: &str, author: Author) -> Result<Oracle> {
    ensure_home(app)?;
    ensure!(valid_slug(slug), "an oracle name uses lowercase letters, digits, - and _ (at most 63)");
    launchpad_core::workspace::get(app, ws).await?;
    check(app, content).await?;
    if plain::write_texts(app, &plain::oracle_store(ws), vec![(path_of(slug), Some(content.to_string()))], author.clone()).await? {
        touch(app, ws, slug, &author).await?;
        changed(app, ws, slug);
    }
    Ok(get(app, ws, slug).await?.oracle)
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

/// Pauses an oracle (it is not evaluated: callers get an error) or resumes it.
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
/// by `author`. The file comes back even when it does not parse, and shows its error. An agent's Restore that widens
/// access is the caller's to refuse (`before` and `widens`).
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

    const FILE: &str = "#!/usr/bin/env python3\n# /// script\n# requires-python = \">=3.11\"\n# dependencies = [\"packaging>=24\"]\n#\n# [tool.launchpad]\n# title = \"CI red\"\n# min_interval = \"2m\"\n# timeout = \"10s\"\n# params = { repo = \"repo\", branch = \"string:main\", pre = \"bool:false\" }\n# access = { tools = [\"gh\"], hosts = [\"PyPI.org\"] }\n# ///\nimport os\n";

    #[test]
    fn parses_the_script_block() {
        let m = parse(FILE).unwrap();
        assert_eq!(m.title, "CI red");
        assert_eq!(m.requires_python.as_deref(), Some(">=3.11"));
        assert_eq!(m.dependencies, vec!["packaging>=24"]);
        assert_eq!((m.min_interval_ms, m.timeout_ms), (120_000, 10_000));
        assert_eq!(m.access, Access { tools: vec!["gh".into()], hosts: vec!["pypi.org".into()], launchpad: LaunchpadRead::None });
        let branch = m.params.iter().find(|p| p.name == "branch").unwrap();
        assert_eq!((branch.kind, branch.default.clone()), (ParamKind::String, Some(Value::String("main".into()))));
        assert_eq!(m.params.iter().find(|p| p.name == "repo").unwrap().default, None);
    }

    #[test]
    fn script_block_edge_cases() {
        assert_eq!(script_block("import os\n").unwrap(), None);
        // The last `# ///` of the comment run closes it; a non-comment line ends the run.
        assert_eq!(script_block("# /// script\n# a = 1\n# ///\n# b = 2\n# ///\nx = 1\n").unwrap().unwrap(), "a = 1\n///\nb = 2\n");
        assert_eq!(script_block("# /// script\r\n# a = 1\r\n#\r\n# ///\r\n").unwrap().unwrap(), "a = 1\n\n");
        assert!(script_block("# /// script\n# a = 1\nx = 1\n# ///\n").is_err());
        assert!(script_block("# /// script\n# ///\nx = 1\n# /// script\n# ///\n").is_err());
        // Other block types are not ours.
        assert_eq!(script_block("# /// pyproject\n# a = 1\n# ///\n").unwrap(), None);
        assert!(parse("# /// script\n# dependencies = []\n# ///\n").unwrap_err().to_string().contains("[tool.launchpad]"));
        assert!(parse("# /// script\n# [tool.launchpad]\n# title = \"x\"\n# timeout = \"2m\"\n# ///\n").is_err());
        assert!(parse("# /// script\n# [tool.launchpad]\n# title = \"x\"\n# min_interval = \"30s\"\n# ///\n").is_err());
        assert!(parse("# /// script\n# [tool.launchpad]\n# title = \"x\"\n# acess = {}\n# ///\n").is_err());
    }

    #[test]
    fn params_convert_and_validate() {
        let specs = parse(FILE).unwrap().params;
        let given = parse_param_args(&["repo=acme/storefront".into(), "pre=yes".into()]).unwrap();
        let v = validate_params(&specs, &given).unwrap();
        assert_eq!(Value::Object(v), serde_json::json!({"repo": "acme/storefront", "branch": "main", "pre": true}));
        assert!(validate_params(&specs, &Map::new()).unwrap_err().to_string().contains("repo is required"));
        assert!(validate_params(&specs, &parse_param_args(&["repo=nope".into()]).unwrap()).is_err());
        assert!(validate_params(&specs, &parse_param_args(&["repo=a/b".into(), "x=1".into()]).unwrap()).is_err());
        assert_eq!(convert(ParamKind::Int, &Value::String(" 42".into())).unwrap(), Value::from(42));
        assert!(parse_param("n", "int:x").is_err());
        assert!(parse_param("N", "int").is_err());
        assert!(parse_param_args(&["a=1".into(), "a=2".into()]).is_err());
    }

    #[test]
    fn access_widening() {
        let old = Access { tools: vec!["gh".into()], hosts: vec!["pypi.org".into()], launchpad: LaunchpadRead::None };
        assert!(widens(Some(&old), &Access { tools: vec![], hosts: vec![], launchpad: LaunchpadRead::None }).is_empty());
        let new = Access { tools: vec!["gh".into(), "hf".into()], hosts: vec!["*".into()], launchpad: LaunchpadRead::Read };
        assert_eq!(widens(Some(&old), &new), vec!["tool hf", "every host (*)", "Launchpad read"]);
        assert_eq!(widens(None, &old), vec!["tool gh", "host pypi.org"]);
    }
}

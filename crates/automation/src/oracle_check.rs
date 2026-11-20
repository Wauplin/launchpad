//! The pre-check of an oracle file (Home): header, Ruff (`ruff check --fix` with Launchpad's fixed rule set, then
//! `ruff format`), the signature of `check`, Monty's parser, then the type checker (warnings only). It runs on every
//! save, every draft run and when an oracle card is filed; what passes is stored and run formatted. Ruff runs as
//! the binary of Home's image on Launchpad's own copy of the text (stdin), with no configuration file.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use launchpad_core::App;
use ruff_python_ast::{Expr, Number, Operator, Stmt, UnaryOp};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use ts_rs::TS;

use crate::oracles::{self, MAX_FILE, Meta, ParamKind, ParamSpec, Problem};

/// Line length of `ruff format`.
pub const LINE_LENGTH: u32 = 112;
/// Ruff's rules: syntax errors, Pyflakes and import sorting, whose fixes Monty can run.
const RULES: &str = "E9,F,I";
/// How long one Ruff run may take.
const RUFF_TIMEOUT: Duration = Duration::from_secs(20);

/// A file the pre-check refused: each problem with its line, column, rule and message.
#[derive(Clone, Debug, Serialize, TS)]
#[ts(rename = "OracleRefused")]
pub struct Refused {
    pub problems: Vec<Problem>,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the pre-check refused the oracle:\n{}", oracles::problems_text(&self.problems))
    }
}

impl std::error::Error for Refused {}

fn refuse(problems: Vec<Problem>) -> anyhow::Error {
    anyhow::Error::new(Refused { problems })
}

/// A file that passed: its formatted text (what Home stores and runs), its definition and its type warnings.
#[derive(Clone, Debug)]
pub struct Checked {
    pub content: String,
    pub meta: Meta,
    pub warnings: Vec<Problem>,
}

/// A binary Home runs for oracles: `$<var>` when set, else `<name>` on PATH (Home's image installs `monty` and
/// `ruff` in /usr/local/bin).
pub fn binary(app: &App, var: &str, name: &str) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os(var).filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let paths = [app.env.get("PATH").map(str::to_string), std::env::var("PATH").ok(), Some("/usr/local/bin:/usr/bin:/bin".to_string())];
    for path in paths.into_iter().flatten() {
        for dir in path.split(':').filter(|d| !d.is_empty()) {
            let p = PathBuf::from(dir).join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Pre-checks a file (`oracles` §3.2): refused (`Refused`) on a header, lint, signature or parse error; otherwise
/// its formatted text, definition and type warnings. An error that is not `Refused` means the check could not run
/// (Ruff missing on Home).
pub async fn precheck(app: &App, content: &str) -> Result<Checked> {
    if content.len() > MAX_FILE {
        return Err(refuse(vec![Problem::new("size", None, "an oracle file is at most 256 KB")]));
    }
    oracles::header(content).map_err(|p| refuse(vec![p]))?;
    nesting(content).map_err(|p| refuse(vec![p]))?;
    let fixed = ruff_fix(app, content).await?;
    let remaining = ruff_lint(app, &fixed).await?;
    if !remaining.is_empty() {
        return Err(refuse(remaining.into_iter().map(|d| d.problem).collect()));
    }
    let formatted = ruff_format(app, &fixed).await?;
    let meta = oracles::meta_of(&formatted).map_err(|p| refuse(vec![p]))?;
    monty_parse(&formatted).map_err(|p| refuse(vec![p]))?;
    let warnings = match crate::oracle_monty::typecheck(app, &formatted).await {
        Ok(Some(text)) => typecheck_problems(&text),
        Ok(None) => vec![],
        Err(e) => vec![Problem { line: None, column: None, rule: Some("type-check".into()), message: format!("the type check did not run: {e:#}") }],
    };
    Ok(Checked { content: formatted, meta, warnings })
}

// ---------------------------------------------------------------- Ruff

async fn ruff(app: &App, args: &[&str], input: &str) -> Result<(i32, String, String)> {
    let bin = binary(app, "LAUNCHPAD_RUFF", "ruff").context("ruff is not installed on Home: oracles are pre-checked with it (Home's image has it; on a dev Home put ruff on PATH or set LAUNCHPAD_RUFF)")?;
    let home = std::env::temp_dir().join(format!("launchpad-ruff-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&home)?;
    let mut cmd = tokio::process::Command::new(bin);
    cmd.args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &home)
        .env("NO_COLOR", "1")
        .current_dir("/")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let run = async {
        let mut child = cmd.spawn().context("could not start ruff")?;
        let mut stdin = child.stdin.take().context("ruff's stdin")?;
        let data = input.as_bytes().to_vec();
        let writer = tokio::spawn(async move {
            let _ = stdin.write_all(&data).await;
            let _ = stdin.shutdown().await;
        });
        let out = child.wait_with_output().await?;
        let _ = writer.await;
        anyhow::Ok((out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned()))
    };
    let r = tokio::time::timeout(RUFF_TIMEOUT, run).await;
    let _ = std::fs::remove_dir_all(&home);
    r.map_err(|_| anyhow::anyhow!("ruff did not answer within {} s", RUFF_TIMEOUT.as_secs()))?
}

fn check_args(extra: &[&'static str]) -> Vec<&'static str> {
    let mut a = vec!["check", "--isolated", "--no-cache", "--select", RULES, "--config", "lint.extend-safe-fixes = [\"F841\"]", "--line-length", "112"];
    a.extend_from_slice(extra);
    a.extend_from_slice(&["--stdin-filename", "oracle.py", "-"]);
    a
}

/// `ruff check --fix` (safe fixes, F841 made safe): the fixed text.
async fn ruff_fix(app: &App, src: &str) -> Result<String> {
    let (code, out, err) = ruff(app, &check_args(&["--fix", "--quiet"]), src).await?;
    if code > 1 || out.is_empty() && !src.is_empty() {
        bail!("ruff check --fix failed: {}", err.trim());
    }
    Ok(out)
}

/// What Ruff still finds in a fixed text: every one refuses the file.
async fn ruff_lint(app: &App, src: &str) -> Result<Vec<RuffDiagnostic>> {
    let (code, out, err) = ruff(app, &check_args(&["--output-format", "json"]), src).await?;
    if code > 1 {
        bail!("ruff check failed: {}", err.trim());
    }
    ruff_json(&out)
}

/// `ruff format` with Launchpad's line length.
async fn ruff_format(app: &App, src: &str) -> Result<String> {
    let (code, out, err) = ruff(app, &["format", "--isolated", "--no-cache", "--line-length", "112", "--stdin-filename", "oracle.py", "-"], src).await?;
    if code != 0 {
        bail!("ruff format failed: {}", err.trim());
    }
    Ok(out)
}

/// The Ruff errors of a stored file that `--fix` would not fix (a broken file): None when Ruff is missing or fails.
pub async fn lint_errors(app: &App, src: &str) -> Option<Vec<Problem>> {
    let (code, out, _) = ruff(app, &check_args(&["--output-format", "json"]), src).await.ok()?;
    if code > 1 {
        return None;
    }
    Some(ruff_json(&out).ok()?.into_iter().filter(|d| !d.fixable).map(|d| d.problem).collect())
}

/// One diagnostic of `ruff check --output-format json`.
#[derive(Clone, Debug, PartialEq)]
pub struct RuffDiagnostic {
    pub problem: Problem,
    /// Ruff has a safe fix for it.
    pub fixable: bool,
}

#[derive(Deserialize)]
struct RuffJson {
    code: Option<String>,
    #[serde(default)]
    name: Option<String>,
    message: String,
    location: Option<RuffLoc>,
    fix: Option<RuffFix>,
}

#[derive(Deserialize)]
struct RuffLoc {
    row: u32,
    column: u32,
}

#[derive(Deserialize)]
struct RuffFix {
    applicability: Option<String>,
}

/// Pure: Ruff's JSON diagnostics. A syntax error has no code (`invalid-syntax`).
pub fn ruff_json(text: &str) -> Result<Vec<RuffDiagnostic>> {
    let list: Vec<RuffJson> = serde_json::from_str(text.trim()).context("ruff's JSON output")?;
    Ok(list
        .into_iter()
        .map(|d| RuffDiagnostic {
            fixable: d.fix.as_ref().is_some_and(|f| f.applicability.as_deref() == Some("safe")),
            problem: Problem { line: d.location.as_ref().map(|l| l.row), column: d.location.as_ref().map(|l| l.column), rule: Some(d.code.or(d.name).unwrap_or_else(|| "invalid-syntax".into())), message: d.message },
        })
        .collect())
}

// ---------------------------------------------------------------- Monty's parser and the signature

/// Refuses a source nested too deeply for the parsers (Monty's own bound), before anything parses it in process.
fn nesting(src: &str) -> std::result::Result<(), Problem> {
    if monty::source_within_nesting_bound(src, 0) { Ok(()) } else { Err(Problem::new("monty", None, "the source is too deeply nested")) }
}

/// Runs a parser on its own thread with a large stack: the parsers recurse, and an async worker's stack is small.
fn on_big_stack<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| std::thread::Builder::new().name("oracle-parse".into()).stack_size(256 << 20).spawn_scoped(scope, f).expect("spawn a parser thread").join().expect("the parser thread"))
}

/// Monty parses and compiles the file (nothing runs): catches what Monty does not support, such as a class with a
/// base class or a `yield`.
pub fn monty_parse(src: &str) -> std::result::Result<(), Problem> {
    on_big_stack(|| monty_parse_here(src))
}

fn monty_parse_here(src: &str) -> std::result::Result<(), Problem> {
    nesting(src)?;
    match monty::MontyRun::new(src.to_string(), "oracle.py", vec![], monty_types::CompileOptions::default()) {
        Ok(_) => Ok(()),
        Err(e) => {
            let frame = e.traceback().last();
            Err(Problem { line: frame.map(|f| f.start.line), column: frame.map(|f| f.start.column), rule: Some("monty".into()), message: e.summary() })
        }
    }
}

/// The parameters of `check`, and whether it takes `cursor` and `now`.
#[derive(Clone, Debug, PartialEq)]
pub struct Signature {
    pub params: Vec<ParamSpec>,
    pub cursor: bool,
    pub now: bool,
}

/// Pure: 1-based line and column of a byte offset.
fn position(src: &str, offset: usize) -> (u32, u32) {
    let before = &src[..offset.min(src.len())];
    let line = before.matches('\n').count() as u32 + 1;
    let col = before.rsplit('\n').next().map(|l| l.chars().count()).unwrap_or(0) as u32 + 1;
    (line, col)
}

fn at(src: &str, range: ruff_text_size::TextRange, message: String) -> Problem {
    let (line, col) = position(src, range.start().to_usize());
    Problem { line: Some(line), column: Some(col), rule: Some("signature".into()), message }
}

enum Annotation {
    Param(ParamKind, bool),
    Datetime,
    Other,
}

fn name_of(e: &Expr) -> Option<&str> {
    match e {
        Expr::Name(n) => Some(n.id.as_str()),
        _ => None,
    }
}

fn annotation(e: &Expr) -> Annotation {
    let kind = |n: &str| match n {
        "str" => Some(ParamKind::Str),
        "int" => Some(ParamKind::Int),
        "float" => Some(ParamKind::Float),
        "bool" => Some(ParamKind::Bool),
        _ => None,
    };
    match e {
        Expr::Name(n) if n.id.as_str() == "datetime" => Annotation::Datetime,
        Expr::Attribute(a) if a.attr.as_str() == "datetime" && name_of(&a.value) == Some("datetime") => Annotation::Datetime,
        Expr::Name(n) => kind(n.id.as_str()).map_or(Annotation::Other, |k| Annotation::Param(k, false)),
        Expr::BinOp(b) if b.op == Operator::BitOr => {
            let one = match (&*b.left, &*b.right) {
                (Expr::NoneLiteral(_), x) | (x, Expr::NoneLiteral(_)) => name_of(x).and_then(kind),
                _ => None,
            };
            one.map_or(Annotation::Other, |k| Annotation::Param(k, true))
        }
        _ => Annotation::Other,
    }
}

/// A default that is a literal: a string, a number, True, False or None.
fn literal(e: &Expr) -> Option<Value> {
    match e {
        Expr::StringLiteral(s) => Some(Value::String(s.value.to_str().to_string())),
        Expr::NumberLiteral(n) => match &n.value {
            Number::Int(i) => i.as_i64().map(Value::from),
            Number::Float(f) => serde_json::Number::from_f64(*f).map(Value::Number),
            Number::Complex { .. } => None,
        },
        Expr::UnaryOp(u) if u.op == UnaryOp::USub => match literal(&u.operand)? {
            Value::Number(n) if n.is_i64() => n.as_i64().map(|v| Value::from(-v)),
            Value::Number(n) => n.as_f64().and_then(|v| serde_json::Number::from_f64(-v)).map(Value::Number),
            _ => None,
        },
        Expr::BooleanLiteral(b) => Some(Value::Bool(b.value)),
        Expr::NoneLiteral(_) => Some(Value::Null),
        _ => None,
    }
}

/// Pure: the signature of the module's `def check(...)` (its last definition): every argument annotated with a
/// supported type (`str`, `int`, `float`, `bool`, or one of them `| None`), defaults that are literals of that type,
/// no `*args` or `**kwargs`; `cursor` (`str | None`) and `now` (`datetime`) without defaults.
pub fn signature(src: &str) -> std::result::Result<Signature, Problem> {
    on_big_stack(|| signature_here(src))
}

fn signature_here(src: &str) -> std::result::Result<Signature, Problem> {
    nesting(src)?;
    let parsed = ruff_python_parser::parse_module(src).map_err(|e| {
        let (line, col) = position(src, e.location.start().to_usize());
        Problem { line: Some(line), column: Some(col), rule: Some("invalid-syntax".into()), message: e.error.to_string() }
    })?;
    let module = parsed.syntax();
    let Some(f) = module.body.iter().rev().find_map(|s| match s {
        Stmt::FunctionDef(f) if f.name.as_str() == "check" => Some(f),
        _ => None,
    }) else {
        return Err(Problem::new("signature", None, "check is missing: define `def check(<parameters>):` at the top level of the file"));
    };
    if f.is_async {
        return Err(at(src, f.range, "check is a plain function, not `async def`".into()));
    }
    let ps = &f.parameters;
    if let Some(v) = &ps.vararg {
        return Err(at(src, v.range, format!("check takes no *{}: name each parameter", v.name.as_str())));
    }
    if let Some(v) = &ps.kwarg {
        return Err(at(src, v.range, format!("check takes no **{}: name each parameter", v.name.as_str())));
    }
    // Home passes every argument by keyword.
    if let Some(p) = ps.posonlyargs.first() {
        return Err(at(src, p.range, "check takes no positional-only parameters (`/`): Home passes every argument by name".into()));
    }
    let mut sig = Signature { params: vec![], cursor: false, now: false };
    for p in ps.args.iter().chain(ps.kwonlyargs.iter()) {
        let name = p.parameter.name.as_str().to_string();
        let Some(ann) = &p.parameter.annotation else {
            return Err(at(src, p.range, format!("parameter {name} needs a type annotation (str, int, float, bool, or one of them | None)")));
        };
        let ann = annotation(ann);
        match name.as_str() {
            "cursor" => {
                if !matches!(ann, Annotation::Param(ParamKind::Str, true)) || p.default.is_some() {
                    return Err(at(src, p.range, "cursor is written `cursor: str | None`, without a default (Home gives it)".into()));
                }
                sig.cursor = true;
            }
            "now" => {
                if !matches!(ann, Annotation::Datetime) || p.default.is_some() {
                    return Err(at(src, p.range, "now is written `now: datetime`, without a default (Home gives it)".into()));
                }
                sig.now = true;
            }
            _ => {
                let Annotation::Param(kind, optional) = ann else {
                    return Err(at(src, p.range, format!("parameter {name}: its type is str, int, float or bool, or one of them | None")));
                };
                let default = match &p.default {
                    None => None,
                    Some(d) => {
                        let v = literal(d).ok_or_else(|| at(src, d.range(), format!("the default of {name} is not a literal (a string, a number, True, False or None)")))?;
                        let fits = match (&v, kind) {
                            (Value::Null, _) => optional,
                            (Value::String(_), ParamKind::Str) | (Value::Bool(_), ParamKind::Bool) => true,
                            (Value::Number(n), ParamKind::Int) => n.is_i64(),
                            (Value::Number(_), ParamKind::Float) => true,
                            _ => false,
                        };
                        if !fits {
                            return Err(at(src, d.range(), format!("the default of {name} is not a {}{}", kind.name(), if optional { " or None" } else { "" })));
                        }
                        Some(if kind == ParamKind::Float { v.as_f64().map(Value::from).unwrap_or(v) } else { v })
                    }
                };
                sig.params.push(ParamSpec { name, kind, optional, required: default.is_none(), default });
            }
        }
    }
    Ok(sig)
}

use ruff_text_size::Ranged;

// ---------------------------------------------------------------- type check

/// Pure: the type checker's concise diagnostics (`oracle.py:10:20: error[rule] message`), as warnings.
pub fn typecheck_problems(text: &str) -> Vec<Problem> {
    let mut out = vec![];
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let mut parts = line.splitn(4, ':');
        let (file, l, c, rest) = (parts.next(), parts.next(), parts.next(), parts.next());
        let parsed = match (file, l.and_then(|x| x.trim().parse::<u32>().ok()), c.and_then(|x| x.trim().parse::<u32>().ok()), rest) {
            (Some(_), Some(l), Some(c), Some(rest)) => {
                let rest = rest.trim();
                let (rule, message) = match (rest.find('['), rest.find("] ")) {
                    (Some(a), Some(b)) if a < b => (Some(rest[a + 1..b].to_string()), rest[b + 2..].to_string()),
                    _ => (None, rest.to_string()),
                };
                Some(Problem { line: Some(l), column: Some(c), rule, message })
            }
            _ => None,
        };
        out.push(parsed.unwrap_or_else(|| Problem { line: None, column: None, rule: None, message: line.to_string() }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(params: &str) -> std::result::Result<Signature, Problem> {
        signature(&format!("from datetime import datetime\n\ndef check({params}):\n    return True\n"))
    }

    #[test]
    fn reads_the_signature() {
        let s = sig("repo: str, branch: str = \"main\", limit: int = 20, hours: float = 2, drafts: bool = False, label: str | None = None, *, n: int = -3").unwrap();
        let names: Vec<_> = s.params.iter().map(|p| (p.name.as_str(), p.kind, p.optional, p.default.clone())).collect();
        assert_eq!(
            names,
            vec![
                ("repo", ParamKind::Str, false, None),
                ("branch", ParamKind::Str, false, Some(Value::from("main"))),
                ("limit", ParamKind::Int, false, Some(Value::from(20))),
                ("hours", ParamKind::Float, false, Some(Value::from(2.0))),
                ("drafts", ParamKind::Bool, false, Some(Value::Bool(false))),
                ("label", ParamKind::Str, true, Some(Value::Null)),
                ("n", ParamKind::Int, false, Some(Value::from(-3))),
            ]
        );
        let s = sig("now: datetime, cursor: str | None, limit: int = 20").unwrap();
        assert!(s.cursor && s.now && s.params.len() == 1);
        assert!(sig("cursor: None | str").unwrap().cursor);
    }

    /// T23: a missing annotation, a non-literal default, **kw, a mistyped cursor and no check are refused at a line.
    #[test]
    fn refuses_unsupported_signatures() {
        for (params, needle) in [
            ("repo, limit: int = 20", "needs a type annotation"),
            ("limit: int = len('ab')", "not a literal"),
            ("**kw", "no **kw"),
            ("*args: str", "no *args"),
            ("cursor: int", "cursor is written"),
            ("cursor: str | None = None", "cursor is written"),
            ("now: str", "now is written"),
            ("limit: int = 1.5", "not a int"),
            ("label: str = None", "not a str"),
            ("items: list", "its type is str"),
            ("x: int = True", "not a int"),
            ("repo: str, /", "positional-only"),
        ] {
            let e = sig(params).unwrap_err();
            assert!(e.message.contains(needle), "{params}: {}", e.message);
            assert_eq!(e.line, Some(3), "{params}");
        }
        assert!(signature("x = 1\n").unwrap_err().message.contains("check is missing"));
        assert!(signature("async def check():\n    pass\n").is_err());
        assert_eq!(signature("def check(:\n").unwrap_err().rule.as_deref(), Some("invalid-syntax"));
    }

    #[test]
    fn monty_parse_refuses_what_monty_lacks() {
        assert!(monty_parse("def check():\n    return True\n").is_ok());
        let e = monty_parse("class B:\n    pass\n\nclass A(B):\n    pass\n").unwrap_err();
        assert_eq!(e.line, Some(4), "{e:?}");
        assert!(monty_parse(&format!("x = {}1{}\n", "(".repeat(5000), ")".repeat(5000))).is_err());
    }

    #[test]
    fn parses_ruff_json() {
        let text = r#"[{"code": "F821", "message": "Undefined name `tool`", "location": {"row": 14, "column": 9}, "fix": null},
            {"code": "F401", "message": "`re` imported but unused", "location": {"row": 4, "column": 8}, "fix": {"applicability": "safe"}},
            {"code": null, "message": "Expected ')'", "location": {"row": 1, "column": 7}, "fix": null}]"#;
        let d = ruff_json(text).unwrap();
        assert_eq!(d[0].problem.text(), "oracle.py:14:9: F821 Undefined name `tool`");
        assert!(!d[0].fixable && d[1].fixable);
        assert_eq!(d[2].problem.rule.as_deref(), Some("invalid-syntax"));
        assert!(ruff_json("[]").unwrap().is_empty());
    }

    #[test]
    fn parses_type_warnings() {
        let w = typecheck_problems("oracle.py:10:20: error[invalid-argument-type] Argument to function `verdict` is incorrect: Expected `bool`, found `Literal[\"yes\"]`\nsomething else\n");
        assert_eq!((w[0].line, w[0].column, w[0].rule.as_deref()), (Some(10), Some(20), Some("invalid-argument-type")));
        assert!(w[0].message.starts_with("Argument to function"));
        assert_eq!((w[1].line, w[1].message.as_str()), (None, "something else"));
    }
}

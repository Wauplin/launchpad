//! `launchpad folio …`: the folios of a workspace (`/api/folios/{workspace}…` on Home). Reads print compact text (an
//! agent reads only what it needs: `outline`, then `read --section`); writes print what Home did and the problems of
//! the check, which runs on every write. `check --dir` and `blocks`/`rules` work offline: the checker and the catalog
//! are in this binary.
//!
//! Exit codes: 0 fine, 1 errors (a check with errors, or with warnings under `--strict`; a write `--strict` refused),
//! 3 a CLI error.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use base64::Engine;
use clap::Subcommand;
use launchpad_automation::folio_catalog as cat;
use launchpad_automation::folio_check::{self as check, Problem};
use launchpad_core::paths::Paths;
use serde_json::{Value, json};

use crate::{Client, query, url_segment, utc};

pub const ERROR_EXIT: i32 = 3;

#[derive(Subcommand)]
pub enum FolioCommand {
    /// The workspace's folios: slug, title, kind, rev, updated, unsent and open marks.
    List {
        #[arg(long)]
        archived: bool,
        #[arg(long)]
        json: bool,
    },
    /// Pages, headings and typed blocks with their ids and lines, problems and open comments.
    Outline {
        slug: String,
        #[arg(long)]
        json: bool,
    },
    /// The raw Markdown of a page (index.md by default), or only one section or the blocks of some items.
    Read {
        slug: String,
        page: Option<String>,
        /// A heading anchor, a container `{#id}` or a typed block or item id.
        #[arg(long)]
        section: Option<String>,
        /// Item ids (comma-separated): the typed blocks that hold them.
        #[arg(long, value_delimiter = ',')]
        items: Vec<String>,
        /// An older rev.
        #[arg(long)]
        rev: Option<i64>,
    },
    /// A new folio from its kind's skeleton (report, roadmap, audit, brief, status, sketchbook).
    Create {
        #[arg(long)]
        title: String,
        #[arg(long, default_value = "report")]
        kind: String,
        #[arg(long)]
        slug: Option<String>,
    },
    /// Write a whole file (stdin, or --file).
    Write {
        slug: String,
        path: String,
        #[arg(long)]
        file: Option<PathBuf>,
        /// The rev you read: a file changed since keeps both versions.
        #[arg(long)]
        rev: Option<i64>,
        /// Refuse the write when it has errors.
        #[arg(long)]
        strict: bool,
    },
    /// Append text to a file (stdin, or --file).
    Append {
        slug: String,
        path: String,
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(long)]
        strict: bool,
    },
    /// Replace one section (a heading to the next of its level, a container or a typed block, by id) with stdin
    /// (or --file): the cheapest update.
    Patch {
        slug: String,
        path: String,
        #[arg(long)]
        section: String,
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(long)]
        rev: Option<i64>,
        #[arg(long)]
        strict: bool,
    },
    /// Add an image or a data file (default path: assets/<file name>).
    AddMedia {
        slug: String,
        file: PathBuf,
        /// Its path in the folio.
        #[arg(long = "as")]
        to: Option<String>,
    },
    /// Remove a file (Restore brings it back).
    Rm { slug: String, path: String },
    /// Check a folio (or a local folder with --dir, offline): file:line, severity, rule, message, hint.
    Check {
        slug: Option<String>,
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Apply the safe fixes (a new rev on Home, or the files of --dir).
        #[arg(long)]
        fix: bool,
        /// Warnings count as errors for the exit code.
        #[arg(long)]
        strict: bool,
        #[arg(long)]
        json: bool,
    },
    /// The block catalog (containers and typed blocks), or one block with its fields and an example.
    Blocks { name: Option<String> },
    /// The check's rules, or one with a bad and a good example.
    Rules { code: Option<String> },
    /// The owner's sent marks (since a cursor), in a few lines.
    Feedback {
        slug: String,
        #[arg(long)]
        since: Option<i64>,
        #[arg(long)]
        json: bool,
    },
    /// Typed items with the owner's marks (JSON).
    Items {
        slug: String,
        #[arg(long = "type")]
        kind: Option<String>,
        #[arg(long)]
        verdict: Option<String>,
        /// Only items with a mark or an open comment.
        #[arg(long)]
        open: bool,
    },
    /// Reply under a comment (`c-31` or `31`).
    Reply {
        slug: String,
        mark: String,
        #[arg(long)]
        text: Option<String>,
    },
    /// Resolve a comment, naming the rev that addressed it (default: the latest).
    Resolve {
        slug: String,
        mark: String,
        #[arg(long)]
        rev: Option<i64>,
    },
    /// Close your round of changes: your revs since your last done become one rev (--message: its label). Run it
    /// once you finish writing, so the folio keeps one rev per round instead of one per write.
    Done {
        slug: String,
        #[arg(long, short)]
        message: Option<String>,
    },
    /// The folio's revs, newest first (named versions show their name).
    Revs { slug: String },
    /// Name a version (the latest rev, or --rev): it is kept for good, past the 30 days of other revs.
    Version {
        slug: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        rev: Option<i64>,
    },
    /// The folio's small key and value store (JSON values; 64 KB per value, 1 MB and 2,000 keys per folio).
    Data {
        #[command(subcommand)]
        command: DataCommand,
    },
}

#[derive(Subcommand)]
pub enum DataCommand {
    /// The keys (with --prefix: only those that start with it) and their values.
    List {
        slug: String,
        #[arg(long, default_value = "")]
        prefix: String,
        #[arg(long)]
        json: bool,
    },
    /// One value, as JSON.
    Get { slug: String, key: String },
    /// Set a key to a JSON value (the argument, else stdin); text that is not JSON is stored as a string.
    Set { slug: String, key: String, value: Option<String> },
    /// Remove a key.
    Rm { slug: String, key: String },
}

pub async fn command(paths: &Paths, workspace: Option<String>, command: FolioCommand) -> Result<i32> {
    match command {
        FolioCommand::Blocks { name } => {
            match name.as_deref() {
                None => {
                    println!("Containers (:::name{{attrs}} title … :::):");
                    for c in cat::CONTAINERS {
                        println!("  :::{:<10} {}", c.name, c.help);
                    }
                    println!("Typed blocks (```name with YAML fields):");
                    for b in cat::BLOCKS {
                        println!("  {:<11} {}{}", b.name, b.help, if b.marks.is_empty() { String::new() } else { format!(" · takes {}", b.marks) });
                    }
                    println!("GitHub alerts (> [!NOTE]) become callouts: {}. Unknown ```x-… blocks render as key and value cards.\nDetails: launchpad folio blocks <name>", cat::ALERTS.join(", "));
                }
                Some(n) => match (cat::block(n.trim_start_matches('`')), cat::container(n.trim_start_matches(':'))) {
                    (Some(b), _) => print!("{}", cat::block_text(b)),
                    (_, Some(c)) => print!("{}", cat::container_text(c)),
                    _ => bail!("no block or container {n:?}: launchpad folio blocks lists them"),
                },
            }
            Ok(0)
        }
        FolioCommand::Rules { code } => {
            match code.as_deref() {
                None => {
                    for r in cat::RULES {
                        println!("{} {:<6} {}", r.code, r.severity.name(), r.title);
                    }
                    println!("Details: launchpad folio rules <code>");
                }
                Some(c) => print!("{}", cat::rule_text(cat::rule(c).with_context(|| format!("no rule {c}"))?)),
            }
            Ok(0)
        }
        FolioCommand::Check { slug: None, dir: Some(dir), fix, strict, json } => check_dir(&dir, fix, strict, json),
        FolioCommand::Check { slug: None, dir: None, .. } => bail!("name a folio, or --dir <folder> to check local files"),
        command => online(paths, workspace, command).await,
    }
}

async fn online(paths: &Paths, workspace: Option<String>, command: FolioCommand) -> Result<i32> {
    let client = Client::new(paths)?;
    let ws = workspace_of(&client, workspace).await?;
    let base = format!("/api/folios/{}", url_segment(&ws));
    let at = |slug: &str, rest: &str| format!("{base}/{}{rest}", url_segment(slug));
    match command {
        FolioCommand::List { archived, json } => {
            let v = client.json("GET", &if archived { format!("{base}?archived=true") } else { base.clone() }, None).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&v)?);
                return Ok(0);
            }
            let list = v.as_array().cloned().unwrap_or_default();
            if list.is_empty() {
                println!("No {}folios in this workspace.", if archived { "archived " } else { "" });
            }
            for f in list {
                println!("{}", list_line(&f));
            }
            Ok(0)
        }
        FolioCommand::Outline { slug, json } => {
            let d = client.json("GET", &at(&slug, ""), None).await?;
            if json {
                let mut d = d;
                for p in d["pages"].as_array_mut().into_iter().flatten() {
                    p.as_object_mut().map(|o| o.remove("text"));
                }
                println!("{}", serde_json::to_string_pretty(&d)?);
            } else {
                print!("{}", outline_text(&d));
            }
            Ok(0)
        }
        FolioCommand::Read { slug, page, section, items, rev } => {
            let page = page.unwrap_or_else(|| "index.md".into());
            let mut pairs = vec![("path", page.as_str())];
            let r = rev.map(|r| r.to_string());
            if let Some(r) = &r {
                pairs.push(("rev", r));
            }
            let (status, text) = client.send("GET", &query(&at(&slug, "/file"), &pairs), None).await?;
            if !(200..300).contains(&status) {
                bail!("{}", serde_json::from_str::<Value>(&text).ok().and_then(|v| v["error"].as_str().map(str::to_string)).unwrap_or(text));
            }
            let head = client.json("GET", &at(&slug, "/revs"), None).await?;
            let current = head.as_array().and_then(|a| a.first()).and_then(|r| r["rev"].as_i64()).unwrap_or(0);
            let shown = rev.unwrap_or(current);
            if let Some(id) = section {
                let range = check::section(&text, &id).with_context(|| format!("{page} has no section `{id}` (it has {})", check::section_ids(&text).join(", ")))?;
                eprintln!("folio {slug} · {page} › {id} · lines {}–{} · rev {shown} (patch it with: launchpad folio patch {slug} {page} --section {id} --rev {shown})", range.0, range.1);
                print!("{}", check::slice(&text, range));
            } else if !items.is_empty() {
                let mut seen = BTreeSet::new();
                for id in &items {
                    let range = check::section(&text, id).with_context(|| format!("no item `{id}` on {page}"))?;
                    if seen.insert(range) {
                        eprintln!("folio {slug} · {page} › {id} · lines {}–{} · rev {shown}", range.0, range.1);
                        print!("{}", check::slice(&text, range));
                    }
                }
            } else {
                eprintln!("folio {slug} · {page} · rev {shown}");
                print!("{text}");
                if !text.ends_with('\n') {
                    println!();
                }
            }
            Ok(0)
        }
        FolioCommand::Create { title, kind, slug } => {
            let w = client.json("POST", &base, Some(json!({"title": title, "kind": kind, "slug": slug}))).await?;
            let slug = w["folio"]["slug"].as_str().unwrap_or("?");
            println!("Created folio {slug} (/folios/{slug}) from the {kind} skeleton: index.md, rev {}.", w["rev"]);
            println!("Next: launchpad folio read {slug}, then write its pages (launchpad folio write {slug} index.md).");
            Ok(written(&w))
        }
        FolioCommand::Write { slug, path, file, rev, strict } => {
            let content = input(file.as_deref())?;
            let w = client.json("POST", &at(&slug, "/write"), Some(json!({"ops": [{"op": "write", "path": path, "content": content}], "base_rev": rev, "strict": strict}))).await?;
            Ok(written(&w))
        }
        FolioCommand::Append { slug, path, file, strict } => {
            let text = input(file.as_deref())?;
            let w = client.json("POST", &at(&slug, "/write"), Some(json!({"ops": [{"op": "append", "path": path, "text": text}], "strict": strict}))).await?;
            Ok(written(&w))
        }
        FolioCommand::Patch { slug, path, section, file, rev, strict } => {
            let text = input(file.as_deref())?;
            let w = client.json("POST", &at(&slug, "/write"), Some(json!({"ops": [{"op": "patch", "path": path, "section": section, "text": text}], "base_rev": rev, "strict": strict}))).await?;
            Ok(written(&w))
        }
        FolioCommand::AddMedia { slug, file, to } => {
            let data = std::fs::read(&file).with_context(|| format!("read {}", file.display()))?;
            let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).context("a file name")?;
            let path = to.unwrap_or_else(|| format!("assets/{name}"));
            let w = client.json("POST", &at(&slug, "/write"), Some(json!({"ops": [{"op": "write", "path": path, "base64": base64::engine::general_purpose::STANDARD.encode(&data)}]}))).await?;
            println!("Reference it as ![…]({path}).");
            Ok(written(&w))
        }
        FolioCommand::Rm { slug, path } => {
            let w = client.json("POST", &at(&slug, "/write"), Some(json!({"ops": [{"op": "remove", "path": path}]}))).await?;
            Ok(written(&w))
        }
        FolioCommand::Check { slug: Some(slug), fix, strict, json, .. } => {
            if fix {
                let w = client.json("POST", &at(&slug, "/fix"), Some(json!({}))).await?;
                return Ok(written(&w));
            }
            let v = client.json("GET", &at(&slug, "/check"), None).await?;
            let problems: Vec<Problem> = serde_json::from_value(v["problems"].clone()).unwrap_or_default();
            Ok(report(&problems, strict, json))
        }
        FolioCommand::Feedback { slug, since, json } => {
            let s = since.map(|s| s.to_string());
            let path = match &s {
                Some(s) => query(&at(&slug, "/feedback"), &[("since", s)]),
                None => at(&slug, "/feedback"),
            };
            let v = client.json("GET", &path, None).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                println!("{}", v["text"].as_str().unwrap_or(""));
                println!("cursor {} (next time: --since {})", v["cursor"], v["cursor"]);
                if v["unsent"].as_u64().unwrap_or(0) > 0 {
                    println!("({} more marks are drafts the owner has not sent)", v["unsent"]);
                }
            }
            Ok(0)
        }
        FolioCommand::Items { slug, kind, verdict, open } => {
            let mut pairs: Vec<(&str, &str)> = vec![];
            if let Some(k) = &kind {
                pairs.push(("type", k));
            }
            if let Some(v) = &verdict {
                pairs.push(("verdict", v));
            }
            if open {
                pairs.push(("open", "true"));
            }
            let v = client.json("GET", &query(&at(&slug, "/items"), &pairs), None).await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
            Ok(0)
        }
        FolioCommand::Reply { slug, mark, text } => {
            let text = match text {
                Some(t) => t,
                None => input(None)?,
            };
            let m = client.json("POST", &at(&slug, &format!("/marks/{}/reply", mark_id(&mark)?)), Some(json!({"text": text}))).await?;
            println!("Replied under c-{} (the owner sees it under the comment).", mark_id(&mark)?);
            let _ = m;
            Ok(0)
        }
        FolioCommand::Resolve { slug, mark, rev } => {
            let m = client.json("POST", &at(&slug, &format!("/marks/{}/resolve", mark_id(&mark)?)), Some(json!({"rev": rev}))).await?;
            println!("Resolved c-{} in rev {}.", m["id"], m["resolved_rev"]);
            Ok(0)
        }
        FolioCommand::Revs { slug } => {
            let v = client.json("GET", &at(&slug, "/revs"), None).await?;
            for r in v.as_array().into_iter().flatten() {
                let files: Vec<String> = r["files"].as_array().into_iter().flatten().map(|f| {
                    let counts = match (f["added"].as_u64(), f["removed"].as_u64()) {
                        (Some(a), Some(d)) => format!(" (+{a} −{d})"),
                        _ => String::new(),
                    };
                    format!("{}{counts}", f["path"].as_str().unwrap_or("?"))
                }).collect();
                let mut label = r["label"].as_str().map(|l| format!(" · {l}")).unwrap_or_default();
                if r["session_id"].is_string() && r["sealed"] == false {
                    label.push_str(" · open");
                }
                let label = match r["version"].as_str() {
                    Some(n) => format!("{label} · version “{n}” (kept)"),
                    None => label,
                };
                println!("rev {} · {} · {}{label} · {}", r["rev"], r["at"].as_i64().map(utc).unwrap_or_default(), r["author"]["name"].as_str().unwrap_or("?"), files.join(", "));
            }
            Ok(0)
        }
        FolioCommand::Done { slug, message } => {
            let w = client.json("POST", &at(&slug, "/done"), Some(json!({"message": message}))).await?;
            for l in w["lines"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                println!("{l}");
            }
            Ok(0)
        }
        FolioCommand::Version { slug, name, rev } => {
            let v = client.json("POST", &at(&slug, "/versions"), Some(json!({"name": name, "rev": rev}))).await?;
            println!("Named rev {} “{}”: kept for good. The owner sees it in Versions.", v["rev"], v["name"].as_str().unwrap_or(""));
            Ok(0)
        }
        FolioCommand::Data { command } => {
            let key_path = |slug: &str, key: &str| at(slug, &format!("/data/{}", key.split('/').map(url_segment).collect::<Vec<_>>().join("/")));
            match command {
                DataCommand::List { slug, prefix, json } => {
                    let v = client.json("GET", &query(&at(&slug, "/data"), &[("prefix", prefix.as_str())]), None).await?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(&v)?);
                        return Ok(0);
                    }
                    let list = v.as_array().cloned().unwrap_or_default();
                    if list.is_empty() {
                        println!("No data{}.", if prefix.is_empty() { String::new() } else { format!(" with the prefix {prefix}") });
                    }
                    for e in list {
                        println!("{}", data_line(&e));
                    }
                }
                DataCommand::Get { slug, key } => {
                    let v = client.json("GET", &key_path(&slug, &key), None).await?;
                    println!("{}", serde_json::to_string_pretty(&v["value"])?);
                }
                DataCommand::Set { slug, key, value } => {
                    let raw = match value {
                        Some(v) => v,
                        None => input(None)?,
                    };
                    let value: Value = serde_json::from_str(raw.trim()).unwrap_or_else(|_| Value::String(raw.trim_end_matches('\n').to_string()));
                    let e = client.json("PUT", &key_path(&slug, &key), Some(json!({"value": value}))).await?;
                    println!("Set {key} ({} bytes).", e["size"]);
                }
                DataCommand::Rm { slug, key } => {
                    let v = client.json("DELETE", &key_path(&slug, &key), None).await?;
                    println!("{}", if v["removed"].as_bool() == Some(true) { format!("Removed {key}.") } else { format!("No key {key}.") });
                }
            }
            Ok(0)
        }
        FolioCommand::Blocks { .. } | FolioCommand::Rules { .. } | FolioCommand::Check { .. } => unreachable!("handled offline"),
    }
}

/// Pure: one line of `folio data list`: the key, its value cut to one line, its size and author.
fn data_line(e: &Value) -> String {
    let v = serde_json::to_string(&e["value"]).unwrap_or_default();
    let v = if v.chars().count() > 80 { format!("{}…", v.chars().take(79).collect::<String>()) } else { v };
    format!("{}  {v}  ({} B, {})", e["key"].as_str().unwrap_or("?"), e["size"], e["author"]["name"].as_str().unwrap_or("?"))
}

/// `c-31` or `31`.
fn mark_id(s: &str) -> Result<i64> {
    s.trim_start_matches("c-").parse().with_context(|| format!("{s:?} is not a comment id (c-31)"))
}

fn input(file: Option<&Path>) -> Result<String> {
    match file {
        Some(f) => std::fs::read_to_string(f).with_context(|| format!("read {}", f.display())),
        None => {
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s).context("read stdin")?;
            Ok(s)
        }
    }
}

/// What a write did, and its problems; 1 when a `--strict` write was refused.
fn written(w: &Value) -> i32 {
    for l in w["lines"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        println!("{l}");
    }
    let problems: Vec<Problem> = serde_json::from_value(w["problems"].clone()).unwrap_or_default();
    for p in &problems {
        println!("{}", p.text());
    }
    if problems.iter().any(|p| p.fix.is_some()) {
        println!("Fixable problems: launchpad folio check {} --fix", w["folio"]["slug"].as_str().unwrap_or("<slug>"));
    }
    if w["written"].as_bool() == Some(false) && w["lines"].as_array().is_some_and(|l| l.iter().any(|x| x.as_str().is_some_and(|s| s.starts_with("Refused")))) { 1 } else { 0 }
}

/// Pure: one line of `folio list`.
fn list_line(f: &Value) -> String {
    let mut facts = vec![format!("{} · rev {}", f["kind"].as_str().unwrap_or("?"), f["rev"])];
    if let Some(t) = f["updated_at"].as_i64() {
        facts.push(format!("updated {}", utc(t)));
    }
    for (k, one, many) in [("unsent", "unsent mark", "unsent marks"), ("open", "open comment", "open comments"), ("errors", "error", "errors")] {
        if let Some(n) = f[k].as_u64().filter(|n| *n > 0) {
            facts.push(format!("{n} {}", if n == 1 { one } else { many }));
        }
    }
    format!("{}  {}  ({})", f["slug"].as_str().unwrap_or("?"), f["title"].as_str().unwrap_or(""), facts.join(" · "))
}

/// Pure: the outline as text.
fn outline_text(d: &Value) -> String {
    let f = &d["folio"];
    let problems: Vec<Problem> = serde_json::from_value(d["problems"].clone()).unwrap_or_default();
    let mut out = format!("folio {} · {} · {} · rev {} · {}\n", f["slug"].as_str().unwrap_or("?"), f["title"].as_str().unwrap_or(""), f["kind"].as_str().unwrap_or(""), d["rev"], check::counts_text(&problems));
    let marks = d["marks"].as_array().cloned().unwrap_or_default();
    for p in d["pages"].as_array().into_iter().flatten() {
        let path = p["path"].as_str().unwrap_or("?");
        out.push_str(&format!("{path}  “{}”\n", p["title"].as_str().unwrap_or("")));
        let text = p["text"].as_str().unwrap_or("");
        let blocks = check::blocks(text);
        for i in p["outline"]["items"].as_array().into_iter().flatten() {
            let line = i["line"].as_u64().unwrap_or(0);
            match i["t"].as_str() {
                Some("heading") => {
                    let id = i["id"].as_str().unwrap_or("");
                    let open = marks.iter().filter(|m| m["kind"] == "comment" && m["resolved_at"].is_null() && m["target"].as_str() == Some(id)).count();
                    out.push_str(&format!("  {line:>4}  {} {}  #{id}{}\n", "#".repeat(i["level"].as_u64().unwrap_or(1) as usize), i["text"].as_str().unwrap_or(""), if open > 0 { format!("  ({open} open comment{})", if open == 1 { "" } else { "s" }) } else { String::new() }));
                }
                Some("container") => out.push_str(&format!("  {line:>4}  :::{}{}\n", i["name"].as_str().unwrap_or(""), i["attrs"].as_str().unwrap_or(""))),
                Some("fence") => {
                    if let Some(b) = blocks.iter().find(|b| b.line as u64 == line) {
                        let ids: Vec<String> = match (b.def, &b.value) {
                            (Some(def), Some(v)) => check::typed_ids(def, v).into_iter().map(|(id, _)| id).collect(),
                            _ => vec![],
                        };
                        out.push_str(&format!("  {line:>4}  ```{}  {}\n", b.lang, if ids.is_empty() { String::new() } else { format!("ids: {}", ids.join(", ")) }));
                    }
                }
                _ => {}
            }
        }
    }
    for p in problems.iter().filter(|p| p.severity != cat::Severity::Info).take(20) {
        out.push_str(&format!("{}\n", p.text()));
    }
    let open: Vec<String> = marks.iter().filter(|m| m["kind"] == "comment" && m["resolved_at"].is_null() && !m["sent_at"].is_null()).map(|m| format!("c-{} {} › #{}", m["id"], m["page"].as_str().unwrap_or("?"), m["target"].as_str().unwrap_or(""))).collect();
    if !open.is_empty() {
        out.push_str(&format!("open comments: {}\n", open.join(", ")));
    }
    out
}

/// Prints problems (text or JSON) and returns the exit code.
fn report(problems: &[Problem], strict: bool, json: bool) -> i32 {
    if json {
        println!("{}", serde_json::to_string_pretty(problems).unwrap_or_default());
    } else {
        for p in problems {
            println!("{}", p.text());
        }
        let fixable = problems.iter().filter(|p| p.fix.is_some()).count();
        println!("{}{} · rules: launchpad folio rules", check::counts_text(problems), if fixable > 0 { format!(" · {fixable} fixable with --fix") } else { String::new() });
    }
    let errors = problems.iter().any(|p| p.severity == cat::Severity::Error);
    let warnings = problems.iter().any(|p| p.severity == cat::Severity::Warn);
    i32::from(errors || (strict && warnings))
}

/// `check --dir`: the same check on local files, offline.
fn check_dir(dir: &Path, fix: bool, strict: bool, json: bool) -> Result<i32> {
    let mut input = check::Input::default();
    fn walk(root: &Path, dir: &Path, input: &mut check::Input) -> Result<()> {
        for e in std::fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
            let e = e?;
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let p = e.path();
            if e.file_type()?.is_dir() {
                walk(root, &p, input)?;
                continue;
            }
            let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            if [".md", ".csv", ".txt"].iter().any(|x| rel.ends_with(x))
                && let Ok(t) = std::fs::read_to_string(&p)
            {
                input.texts.insert(rel.clone(), t);
            }
            input.paths.insert(rel);
        }
        Ok(())
    }
    walk(dir, dir, &mut input)?;
    if !input.texts.keys().any(|p| check::is_page(p)) {
        bail!("{} holds no page (.md)", dir.display());
    }
    let problems = check::check(&input);
    if fix {
        let mut n = 0;
        let by_file: BTreeMap<&String, Vec<Problem>> = input.texts.keys().map(|f| (f, problems.iter().filter(|p| &p.file == f).cloned().collect())).collect();
        for (f, ps) in by_file {
            let (text, k) = check::apply_fixes(&input.texts[f], &ps);
            if k > 0 {
                std::fs::write(dir.join(f), text)?;
                n += k;
            }
        }
        println!("Applied {n} fix{}.", if n == 1 { "" } else { "es" });
        return check_dir(dir, false, strict, json);
    }
    Ok(report(&problems, strict, json))
}

/// The workspace: `--workspace`, else the workspace of this session (`LAUNCHPAD_SESSION_ID`).
async fn workspace_of(client: &Client, workspace: Option<String>) -> Result<String> {
    if let Some(w) = workspace.filter(|w| !w.trim().is_empty()) {
        return Ok(w.trim().to_string());
    }
    let id = std::env::var("LAUNCHPAD_SESSION_ID").ok().filter(|s| !s.is_empty()).context("pass --workspace <workspace id> (outside an agent session)")?;
    let s = client.json("GET", &format!("/api/context/sessions/{}", url_segment(&id)), None).await?;
    s["workspace_id"].as_str().map(str::to_string).context("this session belongs to no workspace: pass --workspace <workspace id>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_and_ids() {
        assert_eq!(mark_id("c-31").unwrap(), 31);
        assert_eq!(mark_id("7").unwrap(), 7);
        assert!(mark_id("x").is_err());
        let f = json!({"slug": "roadmap-winter", "title": "Roadmap", "kind": "roadmap", "rev": 14, "unsent": 5, "open": 0, "errors": 1});
        assert_eq!(list_line(&f), "roadmap-winter  Roadmap  (roadmap · rev 14 · 5 unsent marks · 1 error)");
    }
}

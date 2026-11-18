//! The check of a folio (`launchpad folio check`, and every write): rules F1 (structure), F2 (typed blocks), F3
//! (links and assets), F5 (kind hints) and F6 (size and phone) of `folio_catalog::RULES`, on the files alone, so the
//! CLI runs it offline on a folder (`--dir`) and Home on the stored folio. Problems carry the file, the line, the
//! rule, a message, a hint and, for the fixes with one clear answer, a one-line replacement (`--fix`).
//!
//! Also here (pure): the page's sections for `read --section` and `patch`, the typed items for `items`, the anchors
//! a mark may name (F206) and the skeleton of each kind.
use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};
use serde_yaml_ng::Value as Y;
use ts_rs::TS;

use crate::folio_catalog::{self as cat, Field, Kind, Severity};
use crate::folio_split::{self as split, Item};

/// A one-line fix: replace `from` with `to` on `line`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(rename = "FolioFix")]
pub struct Fix {
    pub line: u32,
    pub from: String,
    pub to: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(rename = "FolioProblem")]
pub struct Problem {
    pub file: String,
    pub line: Option<u32>,
    pub col: Option<u32>,
    pub code: String,
    pub severity: Severity,
    pub message: String,
    pub hint: Option<String>,
    pub fix: Option<Fix>,
}

impl Problem {
    /// `plan.md:42:1    error  F202  decision D-SHARE: missing field `options``.
    pub fn text(&self) -> String {
        let place = match (self.line, self.col) {
            (Some(l), Some(c)) => format!("{}:{l}:{c}", self.file),
            (Some(l), None) => format!("{}:{l}", self.file),
            _ => self.file.clone(),
        };
        let mut s = format!("{place:<16} {:<6} {}  {}", self.severity.name(), self.code, self.message);
        if let Some(h) = &self.hint {
            s.push_str(&format!("; {h}"));
        }
        if self.fix.is_some() {
            s.push_str(" [fixable]");
        }
        s
    }
}

/// "1 error, 2 warnings", "no problems".
pub fn counts_text(problems: &[Problem]) -> String {
    let n = |s: Severity| problems.iter().filter(|p| p.severity == s).count();
    let (e, w, i) = (n(Severity::Error), n(Severity::Warn), n(Severity::Info));
    let mut parts = vec![];
    if e > 0 {
        parts.push(format!("{e} error{}", if e == 1 { "" } else { "s" }));
    }
    if w > 0 {
        parts.push(format!("{w} warning{}", if w == 1 { "" } else { "s" }));
    }
    if i > 0 {
        parts.push(format!("{i} hint{}", if i == 1 { "" } else { "s" }));
    }
    if parts.is_empty() { "no problems".into() } else { parts.join(", ") }
}

/// A mark of the owner that hangs on an id (F206).
#[derive(Clone, Debug)]
pub struct MarkRef {
    pub id: i64,
    pub kind: String,
    pub target: String,
}

/// What the check reads: every file's path, the text of its text files, the folio's marks and, on Home, the slugs
/// of the workspace's folios (links `folio:<slug>`).
#[derive(Default)]
pub struct Input {
    pub texts: BTreeMap<String, String>,
    pub paths: BTreeSet<String>,
    pub marks: Vec<MarkRef>,
    pub slugs: Option<BTreeSet<String>>,
}

impl Input {
    pub fn pages(&self) -> impl Iterator<Item = (&String, &String)> {
        self.texts.iter().filter(|(p, _)| p.ends_with(".md"))
    }
}

/// Is this path a page (a Markdown file)?
pub fn is_page(path: &str) -> bool {
    path.ends_with(".md")
}

struct Ctx<'a> {
    file: &'a str,
    out: Vec<Problem>,
}

impl Ctx<'_> {
    fn push(&mut self, line: Option<u32>, code: &str, message: impl Into<String>, hint: Option<String>, fix: Option<Fix>) {
        let severity = cat::rule(code).map_or(Severity::Warn, |r| r.severity);
        self.out.push(Problem { file: self.file.to_string(), line, col: None, code: code.into(), severity, message: message.into(), hint, fix });
    }
}

/// The front matter of a page as YAML (None: none, or not a mapping).
pub fn front(text: &str) -> Option<serde_yaml_ng::Mapping> {
    let o = split::outline(text);
    let f = o.front?;
    let ls = split::lines(text);
    let body = ls[f.start as usize..f.end as usize - 1].join("\n");
    match serde_yaml_ng::from_str::<Y>(&body).ok()? {
        Y::Mapping(m) => Some(m),
        _ => None,
    }
}

fn s(v: &Y) -> Option<String> {
    match v {
        Y::String(x) => Some(x.clone()),
        Y::Number(n) => Some(n.to_string()),
        Y::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn get<'a>(m: &'a serde_yaml_ng::Mapping, k: &str) -> Option<&'a Y> {
    m.get(Y::String(k.into()))
}

/// The pages of a folio in reading order: `pages:` of index.md (existing ones), then the others by path.
pub fn page_order(texts: &BTreeMap<String, String>) -> Vec<String> {
    let mut order: Vec<String> = vec![];
    if texts.contains_key("index.md") {
        order.push("index.md".into());
    }
    if let Some(m) = texts.get("index.md").and_then(|t| front(t))
        && let Some(Y::Sequence(list)) = get(&m, "pages")
    {
        for p in list.iter().filter_map(s) {
            if texts.contains_key(&p) && !order.contains(&p) {
                order.push(p);
            }
        }
    }
    for p in texts.keys().filter(|p| is_page(p)) {
        if !order.contains(p) {
            order.push(p.clone());
        }
    }
    order
}

/// The ids a page defines that marks may hang on: heading anchors, explicit `{#id}`s and typed block and item ids,
/// each with its line.
#[derive(Default, Debug)]
pub struct Anchors {
    pub headings: Vec<(String, u32)>,
    /// Explicit ids (`{#id}` on containers and headings) and typed ids, with their line and what they are.
    pub ids: Vec<(String, u32, String)>,
    /// renamed_from: old id → new id.
    pub renamed: Vec<(String, String)>,
}

/// A typed block of a page with its parsed YAML (None when it does not parse).
pub struct Block<'a> {
    pub lang: String,
    pub line: u32,
    pub end: u32,
    pub body: String,
    pub value: Option<Y>,
    pub error: Option<(u32, String)>,
    pub def: Option<&'a cat::BlockDef>,
}

/// Typed blocks of a page (catalog names and `x-…`).
pub fn blocks(text: &str) -> Vec<Block<'static>> {
    let ls = split::lines(text);
    split::outline(text)
        .items
        .into_iter()
        .filter_map(|i| match i {
            Item::Fence { line, end, lang, closed, .. } if cat::block(&lang).is_some() || lang.starts_with("x-") => {
                let last = if closed { end as usize - 1 } else { end as usize };
                let body = ls[line as usize..last.max(line as usize)].join("\n");
                let (value, error) = match serde_yaml_ng::from_str::<Y>(&body) {
                    Ok(v) => (Some(v), None),
                    Err(e) => {
                        let l = e.location().map_or(line + 1, |loc| line + loc.line() as u32);
                        (None, Some((l, e.to_string().split(" at line").next().unwrap_or("").to_string())))
                    }
                };
                Some(Block { def: cat::block(&lang), lang, line, end, body, value, error })
            }
            _ => None,
        })
        .collect()
}

/// Lists whose item ids are local to their block (a decision's options, a comparison's options): your choice names
/// the block's id and the option, so they are not folio-wide ids.
pub fn local_ids(block: &str, field: &str) -> bool {
    matches!((block, field), ("decision", "options") | ("options", "items"))
}

/// The typed ids of one parsed block: the block's own id and each item's (not the options of a decision or a
/// comparison), with the item's YAML.
pub fn typed_ids(def: &cat::BlockDef, v: &Y) -> Vec<(String, Y)> {
    let mut out = vec![];
    fn walk(block: &str, fields: &[Field], v: &Y, out: &mut Vec<(String, Y)>) {
        let Y::Mapping(m) = v else { return };
        if fields.iter().any(|f| f.name == "id")
            && let Some(id) = get(m, "id").and_then(s)
        {
            out.push((id, v.clone()));
        }
        for f in fields {
            if local_ids(block, f.name) {
                continue;
            }
            if let (Kind::Items { fields: sub, .. }, Some(Y::Sequence(list))) = (f.kind, get(m, f.name)) {
                for it in list {
                    walk(block, sub, it, out);
                }
            }
        }
    }
    walk(def.name, def.fields, v, &mut out);
    out
}

/// Every anchor of a page.
pub fn anchors(text: &str) -> Anchors {
    let o = split::outline(text);
    let ls = split::lines(text);
    let mut a = Anchors::default();
    for i in &o.items {
        match i {
            Item::Heading { line, id, explicit, .. } => {
                a.headings.push((id.clone(), *line));
                if *explicit {
                    a.ids.push((id.clone(), *line, "anchor".into()));
                }
            }
            Item::Container { line, attrs, .. } => {
                if let Some(id) = split::attrs(attrs).id {
                    a.ids.push((id, *line, "anchor".into()));
                }
            }
            _ => {}
        }
    }
    for b in blocks(text) {
        if let (Some(def), Some(v)) = (b.def, &b.value) {
            for (id, item) in typed_ids(def, v) {
                let line = find_line(&ls, b.line, b.end, &format!("id: {id}")).unwrap_or(b.line);
                if let Y::Mapping(m) = &item
                    && let Some(old) = get(m, "renamed_from")
                {
                    let olds: Vec<String> = match old {
                        Y::Sequence(l) => l.iter().filter_map(s).collect(),
                        other => s(other).into_iter().collect(),
                    };
                    for o in olds {
                        a.renamed.push((o, id.clone()));
                    }
                }
                a.ids.push((id, line, def.name.into()));
            }
        }
    }
    a
}

/// The first line (1-based) in `from..=to` that contains `needle`.
fn find_line(ls: &[&str], from: u32, to: u32, needle: &str) -> Option<u32> {
    (from..=to.min(ls.len() as u32)).find(|l| ls[*l as usize - 1].contains(needle))
}

fn kind_name(k: &Kind) -> &'static str {
    match k {
        Kind::Text => "a text",
        Kind::Id => "an id",
        Kind::Bool => "true or false",
        Kind::Choice(_) => "one of its values",
        Kind::Texts => "a list",
        Kind::Ids => "a list of ids",
        Kind::Items { .. } => "a list of items",
        Kind::Rows => "a list of rows",
    }
}

/// Validates one YAML mapping against its fields; `what` names it in messages (`decision D-SHARE`).
#[allow(clippy::too_many_arguments)]
fn fields(cx: &mut Ctx, ls: &[&str], range: (u32, u32), fs: &[Field], m: &serde_yaml_ng::Mapping, what: &str, markable: bool, deps: &mut Vec<(String, u32)>) {
    let at = |needle: &str| find_line(ls, range.0, range.1, needle);
    let id = get(m, "id").and_then(s);
    let anchor = id.as_deref().map(|i| format!("id: {i}"));
    let line_of_item = anchor.as_deref().and_then(at).unwrap_or(range.0);
    for (k, _) in m {
        let Some(k) = s(k) else { continue };
        if !fs.iter().any(|f| f.name == k) {
            let near = cat::closest(&k, fs.iter().map(|f| f.name));
            let line = find_line(ls, line_of_item, range.1, &format!("{k}:")).or_else(|| at(&format!("{k}:")));
            cx.push(line, "F203", format!("{what}: unknown field `{k}`"), near.map(|n| format!("did you mean `{n}`?")), match (near, line) {
                (Some(n), Some(l)) => Some(Fix { line: l, from: format!("{k}:"), to: format!("{n}:") }),
                _ => None,
            });
        }
    }
    for f in fs {
        let v = get(m, f.name);
        let Some(v) = v.filter(|v| !v.is_null()) else {
            if f.required && !(f.name == "id" && !markable) {
                cx.push(Some(line_of_item), if f.name == "id" { "F207" } else { "F202" }, format!("{what}: missing field `{}`", f.name), Some(f.help.to_string()), None);
            } else if f.name == "id" && markable {
                cx.push(Some(line_of_item), "F207", format!("{what}: no id, so it cannot take your marks"), Some("add an id".into()), None);
            }
            continue;
        };
        let line = find_line(ls, line_of_item, range.1, &format!("{}:", f.name)).unwrap_or(line_of_item);
        let bad = |cx: &mut Ctx, extra: String| cx.push(Some(line), "F204", format!("{what}: `{}` should be {}{extra}", f.name, kind_name(&f.kind)), Some(f.help.to_string()), None);
        match f.kind {
            Kind::Text => {
                if s(v).is_none() {
                    bad(cx, String::new());
                }
            }
            Kind::Id => match s(v) {
                Some(x) if !x.is_empty() && x.chars().all(split::id_char) => {}
                _ => bad(cx, " (letters, digits, -, _, . and :)".into()),
            },
            Kind::Bool => {
                if !matches!(v, Y::Bool(_)) {
                    bad(cx, String::new());
                }
            }
            Kind::Choice(values) => match s(v) {
                Some(x) if values.contains(&x.as_str()) => {}
                Some(x) => {
                    let near = cat::closest(&x, values.iter().copied());
                    cx.push(Some(line), "F204", format!("{what}: `{}: {x}` is not one of {}", f.name, values.join(", ")), near.map(|n| format!("did you mean `{n}`?")), near.map(|n| Fix { line, from: format!("{}: {x}", f.name), to: format!("{}: {n}", f.name) }));
                }
                None => bad(cx, format!(" ({})", values.join(", "))),
            },
            Kind::Texts | Kind::Ids => match v {
                Y::Sequence(l) => {
                    if l.iter().any(|x| s(x).is_none()) {
                        bad(cx, " of plain values".into());
                    }
                    if matches!(f.kind, Kind::Ids) {
                        deps.extend(l.iter().filter_map(s).map(|d| (d, line)));
                    }
                }
                _ if f.name == "renamed_from" && s(v).is_some() => {}
                _ => bad(cx, String::new()),
            },
            Kind::Rows => match v {
                Y::Sequence(rows) if rows.iter().all(|r| matches!(r, Y::Sequence(_))) => {
                    let wide = rows.iter().filter_map(|r| if let Y::Sequence(c) = r { Some(c.len()) } else { None }).max().unwrap_or(0);
                    if wide > 7 {
                        cx.push(Some(line), "F602", format!("{what}: a table with {wide} columns scrolls sideways on the phone"), Some("split it, or use a typed block".into()), None);
                    }
                }
                _ => bad(cx, String::new()),
            },
            Kind::Items { fields: sub, markable: items_markable } => match v {
                Y::Sequence(list) => {
                    let mut recommended = 0;
                    for (n, it) in list.iter().enumerate() {
                        let Y::Mapping(im) = it else {
                            cx.push(Some(line), "F204", format!("{what}: item {} of `{}` should be a mapping (`- {{ id: …, title: … }}`)", n + 1, f.name), None, None);
                            continue;
                        };
                        if matches!(get(im, "recommended"), Some(Y::Bool(true))) {
                            recommended += 1;
                        }
                        let label = get(im, "id").and_then(s).map_or_else(|| format!("{what} › item {}", n + 1), |i| format!("{what} › {i}"));
                        let start = get(im, "id").and_then(s).and_then(|i| find_line(ls, line, range.1, &format!("id: {i}"))).unwrap_or(line);
                        fields(cx, ls, (start, range.1), sub, im, &label, items_markable, deps);
                    }
                    if recommended > 1 {
                        cx.push(Some(line), "F209", format!("{what}: {recommended} options are marked recommended"), Some("recommend at most one".into()), None);
                    }
                }
                _ => bad(cx, String::new()),
            },
        }
    }
}

/// Checks a whole folio.
pub fn check(input: &Input) -> Vec<Problem> {
    let mut out = vec![];
    let pages: Vec<(&String, &String)> = input.pages().collect();
    // Folio-wide ids: id → (file, line).
    let mut ids: HashMap<String, Vec<(String, u32)>> = HashMap::new();
    let mut all_anchors: HashMap<String, Anchors> = HashMap::new();
    let mut deps_all: Vec<(String, String, u32)> = vec![];
    let kind = input.texts.get("index.md").and_then(|t| front(t)).and_then(|m| get(&m, "kind").and_then(s));
    for (path, text) in &pages {
        let a = anchors(text);
        for (id, line, _) in &a.ids {
            ids.entry(id.clone()).or_default().push(((*path).clone(), *line));
        }
        all_anchors.insert((*path).clone(), a);
    }
    for (path, text) in &pages {
        let mut cx = Ctx { file: path, out: vec![] };
        let ls = split::lines(text);
        let o = split::outline(text);
        for p in &o.problems {
            let msg = if p.code == "F101" { "front matter is never closed: end it with a `---` line".to_string() } else {
                match o.items.iter().find(|i| i.line() == p.line) {
                    Some(Item::Fence { lang, .. }) => format!("a code fence{} is never closed: the rest of the page is swallowed", if lang.is_empty() { String::new() } else { format!(" ```{lang}") }),
                    Some(Item::Container { name, .. }) => format!("the container :::{name} is never closed: end it with a `:::` line"),
                    _ => "never closed".to_string(),
                }
            };
            cx.push(Some(p.line), &p.code, msg, None, None);
        }
        // F101: front matter.
        if let Some(f) = &o.front {
            let body = ls[f.start as usize..f.end as usize - 1].join("\n");
            match serde_yaml_ng::from_str::<Y>(&body) {
                Err(e) => cx.push(Some(e.location().map_or(1, |l| l.line() as u32 + 1)), "F101", format!("front matter is not valid YAML: {}", e.to_string().split(" at line").next().unwrap_or("")), None, None),
                Ok(Y::Mapping(m)) => {
                    if path.as_str() == "index.md" && get(&m, "title").and_then(s).is_none_or(|t| t.trim().is_empty()) {
                        cx.push(Some(1), "F101", "index.md has no `title` in its front matter", Some("the title is the folio's name in lists".into()), None);
                    }
                    if path.as_str() == "index.md" && let Some(k) = get(&m, "kind").and_then(s) && !cat::KINDS.iter().any(|(n, _)| *n == k) {
                        let near = cat::closest(&k, cat::KINDS.iter().map(|(n, _)| *n));
                        cx.push(find_line(&ls, 1, f.end, "kind:"), "F104", format!("unknown kind `{k}` ({})", cat::KINDS.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")), near.map(|n| format!("did you mean `{n}`?")), None);
                    }
                    if path.as_str() == "index.md" && let Some(Y::Sequence(list)) = get(&m, "pages") {
                        let line = find_line(&ls, 1, f.end, "pages:");
                        for p in list.iter().filter_map(s) {
                            if !input.texts.contains_key(&p) {
                                cx.push(line, "F105", format!("pages: names `{p}`, which does not exist"), None, None);
                            }
                        }
                        let listed: Vec<String> = list.iter().filter_map(s).collect();
                        for (other, _) in &pages {
                            if other.as_str() != "index.md" && !listed.contains(other) {
                                cx.push(line, "F105", format!("the page `{other}` is not listed in pages:, so readers cannot reach it"), Some(format!("add {other} to pages:")), None);
                            }
                        }
                    }
                }
                Ok(Y::Null) => {
                    if path.as_str() == "index.md" {
                        cx.push(Some(1), "F101", "index.md has no `title` in its front matter", None, None);
                    }
                }
                Ok(_) => cx.push(Some(1), "F101", "front matter is not a mapping of keys", None, None),
            }
        } else if path.as_str() == "index.md" && !o.problems.iter().any(|p| p.code == "F101") {
            cx.push(Some(1), "F101", "index.md has no front matter with a `title`", Some("start it with ---, title: …, kind: …, ---".into()), None);
        }
        // Headings: duplicates (F106) and level jumps (F502).
        let mut last_level = 0u8;
        for i in &o.items {
            if let Item::Heading { line, level, id, text: t, explicit } = i {
                if !explicit && id.rsplit_once('-').is_some_and(|(base, n)| n.parse::<u32>().is_ok() && o.items.iter().any(|j| matches!(j, Item::Heading { id: other, .. } if other == base))) {
                    cx.push(Some(*line), "F106", format!("the heading “{t}” gets the anchor `{id}`, like an earlier one"), Some("give it its own {#id}".into()), Some(Fix { line: *line, from: t.clone(), to: format!("{t} {{#{id}}}") }));
                }
                if last_level > 0 && *level > last_level + 1 {
                    cx.push(Some(*line), "F502", format!("heading level jumps from {} to {}", last_level, level), None, None);
                }
                last_level = *level;
            }
        }
        // Containers: names (F103) and attributes (F104).
        for i in &o.items {
            if let Item::Container { line, name, attrs, .. } = i {
                let near_def = cat::closest(name, cat::CONTAINERS.iter().map(|c| c.name)).and_then(cat::container);
                if cat::container(name).is_none() {
                    {
                        let near = cat::closest(name, cat::CONTAINERS.iter().map(|c| c.name));
                        let hint = match (near, name.as_str()) {
                            (Some(n), _) => Some(format!("did you mean `:::{n}`?")),
                            (None, "warning" | "warn" | "note" | "tip" | "info" | "danger" | "risk" | "important" | "caution") => Some(format!("use `:::callout{{tone={}}}`", match name.as_str() { "warning" | "danger" | "caution" => "warn", "important" => "verdict", "tip" => "tip", "note" => "note", n => n })),
                            _ => None,
                        };
                        cx.push(Some(*line), "F103", format!("unknown container `:::{name}`: it renders as a plain box"), hint, near.map(|n| Fix { line: *line, from: format!(":{name}"), to: format!(":{n}") }));
                    }
                }
                // The attributes of a misspelled container are checked as the container it would become.
                if let Some(def) = cat::container(name).or(near_def) {
                    {
                        let a = split::attrs(attrs);
                        for (k, v) in a.pairs.iter().map(|(k, v)| (k.as_str(), Some(v.as_str()))).chain(a.flags.iter().map(|f| (f.as_str(), None))) {
                            match def.attrs.iter().find(|(n, _)| *n == k) {
                                None if k == "title" => {}
                                None => cx.push(Some(*line), "F104", format!(":::{name} takes no attribute `{k}`"), (!def.attrs.is_empty()).then(|| format!("it takes {}", def.attrs.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", "))), None),
                                Some((_, values)) => {
                                    if let Some(v) = v && !values.is_empty() && !values.contains(&v) {
                                        let near = cat::closest(v, values.iter().copied());
                                        cx.push(Some(*line), "F104", format!(":::{name}: `{k}={v}` is not one of {}", values.join(", ")), near.map(|n| format!("did you mean `{k}={n}`?")), near.map(|n| Fix { line: *line, from: format!("{k}={v}"), to: format!("{k}={n}") }));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        // Typed blocks (F2) and code lines (F602).
        let typed: Vec<Block> = blocks(text);
        let mut verdict = false;
        for i in &o.items {
            if let Item::Container { name, attrs, .. } = i && name == "callout" && split::attrs(attrs).get("tone") == Some("verdict") {
                verdict = true;
            }
            if let Item::Fence { line, end, lang, .. } = i
                && cat::block(lang).is_none()
                && !lang.starts_with("x-")
            {
                {
                    if let Some(near) = cat::closest(lang, cat::BLOCKS.iter().map(|b| b.name)).filter(|_| lang.len() >= 5) {
                        cx.push(Some(*line), "F208", format!("```{lang} is not a block type: it renders as code"), Some(format!("did you mean ```{near}?")), Some(Fix { line: *line, from: lang.clone(), to: near.to_string() }));
                    }
                    for l in *line + 1..*end {
                        if ls.get(l as usize - 1).is_some_and(|x| x.chars().count() > 120) {
                            cx.push(Some(l), "F602", "a code line over 120 characters scrolls sideways on the phone", None, None);
                            break;
                        }
                    }
                }
            }
        }
        for b in &typed {
            let Some(def) = b.def else {
                cx.push(Some(b.line), "F208", format!("unknown block type ```{}: it renders as a key and value card", b.lang), Some("see `launchpad folio blocks` for the catalog".into()), None);
                continue;
            };
            if let Some((l, e)) = &b.error {
                cx.push(Some(*l), "F201", format!("{}: the YAML does not parse: {e}", b.lang), None, None);
                continue;
            }
            let what = match b.value.as_ref() {
                Some(Y::Mapping(m)) => get(m, "id").and_then(s).map_or(def.name.to_string(), |i| format!("{} {i}", def.name)),
                _ => def.name.to_string(),
            };
            match &b.value {
                Some(Y::Mapping(m)) => {
                    let mut deps = vec![];
                    fields(&mut cx, &ls, (b.line, b.end), def.fields, m, &what, def.markable, &mut deps);
                    deps_all.extend(deps.into_iter().map(|(d, l)| ((*path).clone(), d, l)));
                    if kind.as_deref() == Some("audit") && def.name == "findings" && let Some(Y::Sequence(items)) = get(m, "items") && items.iter().any(|it| matches!(it, Y::Mapping(im) if get(im, "severity").is_none())) {
                        cx.push(Some(b.line), "F501", "an audit's findings usually have a severity", None, None);
                    }
                    if def.name == "table" {
                        table(&mut cx, &ls, b, m, input);
                    }
                }
                _ => cx.push(Some(b.line), "F202", format!("{}: the block is empty or not a mapping of fields", def.name), Some(format!("see `launchpad folio blocks {}`", def.name)), None),
            }
        }
        if path.as_str() == "index.md" && kind.as_deref() == Some("roadmap") && !verdict {
            cx.push(None, "F501", "a roadmap usually opens with a verdict (:::callout{tone=verdict})", None, None);
        }
        // Alerts, links, images and GFM tables, outside fences.
        let fenced: Vec<(u32, u32)> = o.items.iter().filter_map(|i| if let Item::Fence { line, end, .. } = i { Some((*line, *end)) } else { None }).collect();
        let front_end = o.front.as_ref().map_or(0, |f| f.end);
        for (n, l) in ls.iter().enumerate() {
            let no = n as u32 + 1;
            if no <= front_end || fenced.iter().any(|(a, b)| no >= *a && no <= *b) {
                continue;
            }
            let t = l.trim_start();
            if let Some(rest) = t.strip_prefix("> [!")
                && let Some(end) = rest.find(']')
            {
                let kind = &rest[..end];
                if !cat::ALERTS.contains(&kind.to_uppercase().as_str()) {
                    let near = cat::closest(kind, cat::ALERTS.iter().copied());
                    cx.push(Some(no), "F107", format!("unknown alert type [!{kind}]"), near.map(|x| format!("did you mean [!{x}]?")), near.map(|x| Fix { line: no, from: format!("[!{kind}]"), to: format!("[!{x}]") }));
                }
            }
            if t.starts_with('|') {
                let cells = t.trim().trim_matches('|').split('|').count();
                if cells > 7 && !ls.get(n.wrapping_sub(1)).is_some_and(|p| p.trim_start().starts_with('|')) {
                    cx.push(Some(no), "F602", format!("a table with {cells} columns scrolls sideways on the phone"), Some("split it, or use a typed block".into()), None);
                }
            }
            for (image, target) in links(l) {
                link(&mut cx, no, image, &target, path, input, &all_anchors);
            }
        }
        if text.len() > 60 * 1024 {
            cx.push(None, "F601", format!("the page is {} KB: split it into pages", text.len() / 1024), None, None);
        }
        out.extend(cx.out);
    }
    // F205: duplicate ids across the folio.
    let mut dups: Vec<(&String, &Vec<(String, u32)>)> = ids.iter().filter(|(_, v)| v.len() > 1).collect();
    dups.sort();
    for (id, places) in dups {
        let list = places.iter().map(|(f, l)| format!("{f}:{l}")).collect::<Vec<_>>().join(", ");
        let (f, l) = &places[1];
        out.push(Problem { file: f.clone(), line: Some(*l), col: None, code: "F205".into(), severity: Severity::Error, message: format!("id `{id}` is used {} times ({list})", places.len()), hint: Some("marks need unique ids".into()), fix: None });
    }
    // F209: deps that point nowhere.
    for (file, d, l) in deps_all {
        if !ids.contains_key(&d) {
            out.push(Problem { file, line: Some(l), col: None, code: "F209".into(), severity: Severity::Warn, message: format!("deps names `{d}`, which no item of the folio has"), hint: None, fix: None });
        }
    }
    // F206: marks whose id is gone.
    let mut orphans: BTreeMap<String, Vec<&MarkRef>> = BTreeMap::new();
    let known = |t: &str| ids.contains_key(t) || all_anchors.values().any(|a| a.headings.iter().any(|(h, _)| h == t) || a.renamed.iter().any(|(o, _)| o == t));
    for m in &input.marks {
        if !m.target.is_empty() && !known(&m.target) {
            orphans.entry(m.target.clone()).or_default().push(m);
        }
    }
    for (target, marks) in orphans {
        let list = marks.iter().map(|m| if m.kind == "comment" { format!("comment c-{}", m.id) } else { m.kind.clone() }).collect::<Vec<_>>().join(", ");
        out.push(Problem { file: "index.md".into(), line: None, col: None, code: "F206".into(), severity: Severity::Warn, message: format!("`{target}` is gone: {} of your marks point to it ({list})", marks.len()), hint: Some(format!("if it was renamed, add `renamed_from: [{target}]` to the new item")), fix: None });
    }
    out.sort_by(|a, b| (a.file.as_str(), a.line.unwrap_or(0)).cmp(&(b.file.as_str(), b.line.unwrap_or(0))));
    out
}

/// A table block: its CSV exists (F302) and has the columns it names (F304).
fn table(cx: &mut Ctx, ls: &[&str], b: &Block, m: &serde_yaml_ng::Mapping, input: &Input) {
    let Some(src) = get(m, "src").and_then(s) else {
        if get(m, "rows").is_none() {
            cx.push(Some(b.line), "F202", "table: give `rows` or `src` (a CSV file of the folio)", None, None);
        }
        return;
    };
    let line = find_line(ls, b.line, b.end, "src:");
    if !input.paths.contains(&src) {
        cx.push(line, "F302", format!("table: `{src}` is not a file of the folio"), Some(format!("add it with launchpad folio write <slug> {src}")), None);
        return;
    }
    let Some(csv) = input.texts.get(&src) else { return };
    let header: Vec<String> = csv.lines().next().unwrap_or("").split(',').map(|c| c.trim().trim_matches('"').to_string()).collect();
    if let Some(Y::Sequence(cols)) = get(m, "columns") {
        for c in cols.iter().filter_map(s) {
            if !header.contains(&c) {
                cx.push(find_line(ls, b.line, b.end, "columns:"), "F304", format!("table: `{src}` has no column `{c}` (it has {})", header.join(", ")), None, None);
            }
        }
    }
}

/// The links and images of a line: `(is_image, target)`, outside inline code.
pub fn links(line: &str) -> Vec<(bool, String)> {
    let mut out = vec![];
    let b = line.as_bytes();
    let mut i = 0;
    let mut code = false;
    while i < b.len() {
        match b[i] {
            b'`' => code = !code,
            b']' if !code && b.get(i + 1) == Some(&b'(') => {
                // Find the matching `[`.
                let mut depth = 0;
                let mut start = None;
                for j in (0..i).rev() {
                    match b[j] {
                        b']' => depth += 1,
                        b'[' if depth == 0 => {
                            start = Some(j);
                            break;
                        }
                        b'[' => depth -= 1,
                        _ => {}
                    }
                }
                if let Some(st) = start
                    && let Some(close) = line[i + 2..].find(')')
                {
                    let target = line[i + 2..i + 2 + close].split_whitespace().next().unwrap_or("").to_string();
                    out.push((st > 0 && b[st - 1] == b'!', target));
                    i += 2 + close;
                }
            }
            _ => {}
        }
        i += 1;
    }
    out
}

fn link(cx: &mut Ctx, line: u32, image: bool, target: &str, page: &str, input: &Input, anchors: &HashMap<String, Anchors>) {
    if target.is_empty() || target.starts_with("mailto:") {
        return;
    }
    let network = target.starts_with("http://") || target.starts_with("https://") || target.starts_with("//");
    if image {
        if network {
            cx.push(Some(line), "F303", format!("image from the network ({target}): the reader does not load it"), Some("add it with launchpad folio add-media".into()), None);
        } else if !target.starts_with("data:") && !input.paths.contains(&resolve(page, target.split('#').next().unwrap_or(""))) {
            cx.push(Some(line), "F302", format!("image `{target}` is not a file of the folio"), Some("add it with launchpad folio add-media".into()), None);
        }
        return;
    }
    if network || target.contains("://") {
        return;
    }
    if let Some(rest) = target.strip_prefix("folio:") {
        let slug = rest.split('#').next().unwrap_or("");
        if let Some(slugs) = &input.slugs
            && !slugs.contains(slug)
        {
            cx.push(Some(line), "F301", format!("link to folio `{slug}`, which this workspace does not have"), None, None);
        }
        return;
    }
    let (file, anchor) = match target.split_once('#') {
        Some((f, a)) => (f, Some(a)),
        None => (target, None),
    };
    let file = if file.is_empty() { page.to_string() } else { resolve(page, file) };
    if !input.paths.contains(&file) {
        let near = cat::closest(&file, input.paths.iter().map(String::as_str));
        cx.push(Some(line), "F301", format!("link to `{file}`, which is not a file of the folio"), near.map(|n| format!("did you mean `{n}`?")), None);
        return;
    }
    if let (Some(a), Some(an)) = (anchor, anchors.get(&file))
        && !a.is_empty()
        && !an.headings.iter().any(|(h, _)| h == a)
        && !an.ids.iter().any(|(h, _, _)| h == a)
    {
        cx.push(Some(line), "F301", format!("link to `{file}#{a}`: that page has no anchor `{a}`"), None, None);
    }
}

/// A relative path from a page, normalized (`../x` from `a/b.md` → `x`).
pub fn resolve(page: &str, target: &str) -> String {
    let target = target.trim_start_matches("./");
    if target.starts_with('/') {
        return target.trim_start_matches('/').to_string();
    }
    let mut parts: Vec<&str> = page.rsplit_once('/').map(|(d, _)| d.split('/').collect()).unwrap_or_default();
    for seg in target.split('/') {
        match seg {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// Applies the fixes of the problems of one file; returns the new text and how many applied.
pub fn apply_fixes(text: &str, problems: &[Problem]) -> (String, usize) {
    let mut ls: Vec<String> = split::lines(text).into_iter().map(str::to_string).collect();
    let mut n = 0;
    for p in problems {
        if let Some(f) = &p.fix
            && let Some(l) = ls.get_mut(f.line as usize - 1)
            && l.contains(&f.from)
        {
            *l = l.replacen(&f.from, &f.to, 1);
            n += 1;
        }
    }
    let mut out = ls.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    (out, n)
}

// ---------------------------------------------------------------- sections

/// The line range (1-based, inclusive) of a section: a heading with that anchor (to the next heading of the same or
/// a higher level), a container with that `{#id}`, or a typed block whose `id` (or one of whose items) has it.
pub fn section(text: &str, id: &str) -> Option<(u32, u32)> {
    let o = split::outline(text);
    let total = split::lines(text).len() as u32;
    for (n, i) in o.items.iter().enumerate() {
        match i {
            Item::Heading { line, level, id: h, .. } if h == id => {
                let end = o.items[n + 1..].iter().find_map(|j| match j {
                    Item::Heading { line: l, level: lv, .. } if lv <= level => Some(l - 1),
                    _ => None,
                });
                // A heading inside a container ends with it.
                let container_end = o.items[..n].iter().filter_map(|j| match j {
                    Item::Container { line: a, end: b, .. } if a < line && b > line => Some(b - 1),
                    _ => None,
                }).min();
                let end = [end, container_end].into_iter().flatten().min().unwrap_or(total);
                return Some((*line, end));
            }
            Item::Container { line, end, attrs, .. } if split::attrs(attrs).id.as_deref() == Some(id) => return Some((*line, *end)),
            _ => {}
        }
    }
    for b in blocks(text) {
        if let (Some(def), Some(v)) = (b.def, &b.value)
            && typed_ids(def, v).iter().any(|(i, _)| i == id)
        {
            return Some((b.line, b.end));
        }
    }
    None
}

/// The text of lines `a..=b`.
pub fn slice(text: &str, (a, b): (u32, u32)) -> String {
    let ls = split::lines(text);
    let mut out = ls[a as usize - 1..(b as usize).min(ls.len())].join("\n");
    out.push('\n');
    out
}

/// Replaces lines `a..=b` with `with` (ends with a newline).
pub fn replace(text: &str, (a, b): (u32, u32), with: &str) -> String {
    let ls = split::lines(text);
    let mut out: Vec<String> = ls[..a as usize - 1].iter().map(|s| s.to_string()).collect();
    out.extend(split::lines(with).into_iter().map(str::to_string));
    out.extend(ls[(b as usize).min(ls.len())..].iter().map(|s| s.to_string()));
    let mut t = out.join("\n");
    t.push('\n');
    t
}

/// Every id a section can be read or patched by on this page (for "no section `x`" errors).
pub fn section_ids(text: &str) -> Vec<String> {
    let a = anchors(text);
    let mut v: Vec<String> = a.headings.into_iter().map(|(h, _)| h).chain(a.ids.into_iter().map(|(i, _, _)| i)).collect();
    v.dedup();
    v
}

// ---------------------------------------------------------------- skeletons

/// A new folio's index.md for a kind.
pub fn skeleton(title: &str, kind: &str) -> String {
    let title_line = serde_json::to_string(title).unwrap_or_default();
    let head = format!("---\ntitle: {title_line}\nkind: {kind}\nstatus: draft\nsummary: \"\"\npages: [index.md]\n---\n\n");
    let body = match kind {
        "roadmap" => ":::callout{tone=verdict} Verdict\nThe recommendation in two sentences.\n:::\n\n## Waves {#waves}\n\n```roadmap\nwaves:\n  - id: W1\n    title: First wave\n    items:\n      - { id: F-01, title: First item, advice: do }\n```\n\n## Decisions {#decisions}\n\n```decision\nid: D-1\nquestion: The open question?\noptions:\n  - { id: a, label: Option A, recommended: true }\n  - { id: b, label: Option B }\n```\n",
        "audit" => ":::callout{tone=verdict} Verdict\nWhat the audit found, in two sentences.\n:::\n\n## Findings {#findings}\n\n```findings\nitems:\n  - id: A-01\n    title: First finding\n    severity: medium\n    where: path/to/file\n    fix: The fix.\n```\n",
        "brief" => "## What happened {#what-happened}\n\n- …\n\n## What waits for you {#waits}\n\n```checklist\nitems:\n  - { id: C-1, text: First thing to check }\n```\n",
        "status" => "## Status {#status}\n\n```status\ntiles:\n  - { area: First area, state: ok, text: One line }\n```\n\n## Timeline {#timeline}\n\n```timeline\nitems:\n  - { date: 2026-01-01, title: Started }\n```\n",
        "sketchbook" => "## Options {#options}\n\n```options\nid: O-1\nquestion: Which layout?\nitems:\n  - { id: a, label: Layout A, pros: [simple], cons: [dense] }\n  - { id: b, label: Layout B }\n```\n",
        _ => ":::callout{tone=verdict} Verdict\nThe answer in two sentences.\n:::\n\n## Decisions {#decisions}\n\n```decisions\nrows:\n  - { question: The question, decision: The decision, effect: Its effect }\n```\n\n## Details {#details}\n\n…\n\n## Sources {#sources}\n\n```sources\nitems:\n  - { title: A source, url: \"https://example.com\" }\n```\n",
    };
    format!("{head}{body}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(files: &[(&str, &str)]) -> Input {
        Input { texts: files.iter().map(|(p, t)| (p.to_string(), t.to_string())).collect(), paths: files.iter().map(|(p, _)| p.to_string()).collect(), ..Default::default() }
    }

    fn codes(p: &[Problem]) -> Vec<&str> {
        p.iter().map(|x| x.code.as_str()).collect()
    }

    #[test]
    fn skeletons_and_examples_pass() {
        for (k, _) in cat::KINDS {
            let p = check(&input(&[("index.md", &skeleton("Winter roadmap", k))]));
            assert!(p.is_empty(), "{k}: {p:?}");
        }
        for b in cat::BLOCKS {
            let page = format!("---\ntitle: T\nkind: report\n---\n\n:::callout{{tone=verdict}}\nx\n:::\n\n```{}\n{}```\n", b.name, b.example);
            let p = check(&input(&[("index.md", &page)]));
            assert!(p.is_empty(), "{}: {p:?}", b.name);
        }
    }

    #[test]
    fn typed_block_rules() {
        let page = "---\ntitle: T\nkind: roadmap\n---\n\n```decision\nid: D-SHARE\nquestion: Expire?\noption: [a, b]\n```\n\n```roadmap\nwaves:\n  - id: W1\n    title: One\n    size: huge\n    items:\n      - { id: F-07, title: A, advise: do }\n      - { id: F-07, title: B, deps: [W4] }\n      - { title: C }\n```\n\n```x-poll\nq: 1\n```\n\n```findings\nitems: [\n```\n";
        let p = check(&input(&[("index.md", page)]));
        let c = codes(&p);
        for want in ["F202", "F203", "F204", "F205", "F207", "F208", "F209", "F201", "F501"] {
            assert!(c.contains(&want), "{want} missing: {p:#?}");
        }
        let f203 = p.iter().find(|x| x.code == "F203" && x.message.contains("advise")).unwrap();
        assert_eq!(f203.hint.as_deref(), Some("did you mean `advice`?"));
        let (fixed, n) = apply_fixes(page, &p);
        assert!(n >= 1 && fixed.contains("advice: do") && fixed.contains("size: huge"));
        let f202 = p.iter().find(|x| x.code == "F202").unwrap();
        assert_eq!((f202.line, f202.message.as_str()), (Some(7), "decision D-SHARE: missing field `options`"));
        let f201 = p.iter().find(|x| x.code == "F201").unwrap();
        assert!(f201.line.unwrap() >= 27, "{f201:?}");
    }

    #[test]
    fn structure_and_link_rules() {
        let index = "---\ntitle: T\nkind: report\npages: [index.md, missing.md]\n---\n\n:::warning\nx\n:::\n\n:::calout{tone=verdikt}\nx\n:::\n\n> [!NOTICE]\n> x\n\n## A\n#### B\n## A\n\n[plan](plan.md#nope) [ok](plan.md#waves) ![img](assets/x.png) ![net](https://e.com/x.png) [f](folio:other)\n\n| a | b | c | d | e | f | g | h |\n";
        let plan = "## Waves\n\n```python\nprint('x')\n```\n";
        let mut i = input(&[("index.md", index), ("plan.md", plan), ("orphan.md", "# O\n")]);
        i.slugs = Some(["mine".to_string()].into());
        let p = check(&i);
        let c = codes(&p);
        for want in ["F103", "F104", "F105", "F106", "F107", "F301", "F302", "F303", "F502", "F602"] {
            assert!(c.contains(&want), "{want} missing: {p:#?}");
        }
        assert_eq!(p.iter().filter(|x| x.code == "F105").count(), 3, "{p:#?}");
        assert_eq!(p.iter().filter(|x| x.code == "F301").count(), 2, "{p:#?}");
        let (fixed, _) = apply_fixes(index, &p.iter().filter(|x| x.file == "index.md").cloned().collect::<Vec<_>>());
        assert!(fixed.contains(":::callout{tone=verdict}") && fixed.contains("[!NOTE]"), "{fixed}");
    }

    #[test]
    fn marks_and_renames() {
        let page = "---\ntitle: T\nkind: report\n---\n\n## Waves\n\n```roadmap\nwaves:\n  - id: W1\n    title: One\n    items:\n      - { id: F-09, title: A, renamed_from: [F-03] }\n```\n";
        let mut i = input(&[("index.md", page)]);
        i.marks = vec![MarkRef { id: 1, kind: "verdict".into(), target: "F-03".into() }, MarkRef { id: 2, kind: "comment".into(), target: "F-04".into() }, MarkRef { id: 3, kind: "comment".into(), target: "waves".into() }];
        let p = check(&i);
        let f206: Vec<&Problem> = p.iter().filter(|x| x.code == "F206").collect();
        assert_eq!(f206.len(), 1, "{p:#?}");
        assert!(f206[0].message.contains("F-04") && f206[0].message.contains("comment c-2"));
        assert_eq!(anchors(page).renamed, vec![("F-03".to_string(), "F-09".to_string())]);
    }

    #[test]
    fn sections() {
        let page = "# Top\n\nIntro\n\n## Waves\n\ntext\n\n```decision\nid: D-1\nquestion: Q?\noptions:\n  - { id: a, label: A }\n```\n\n### Sub\n\n## Next\n\nend\n";
        assert_eq!(section(page, "waves"), Some((5, 17)));
        assert_eq!(section(page, "D-1"), Some((9, 14)));
        assert_eq!(section(page, "a"), None, "option ids are local to their decision");
        assert_eq!(section(page, "next"), Some((18, 20)));
        assert_eq!(section(page, "nope"), None);
        let patched = replace(page, section(page, "next").unwrap(), "## Next\n\nnew\n");
        assert!(patched.ends_with("## Next\n\nnew\n") && patched.contains("### Sub"));
        assert_eq!(slice(page, (1, 1)), "# Top\n");
    }

    #[test]
    fn page_order_follows_pages() {
        let i = input(&[("index.md", "---\ntitle: T\npages: [index.md, b.md, a.md]\n---\n"), ("a.md", ""), ("b.md", ""), ("c.md", "")]);
        assert_eq!(page_order(&i.texts), vec!["index.md", "b.md", "a.md", "c.md"]);
        assert_eq!(resolve("dir/page.md", "../x.png"), "x.png");
        assert_eq!(resolve("page.md", "./assets/a.png"), "assets/a.png");
        assert_eq!(links("see [a](b.md) and `[c](d)` ![i](x.png \"t\")"), vec![(false, "b.md".to_string()), (true, "x.png".to_string())]);
    }
}

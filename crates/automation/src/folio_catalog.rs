//! The folio format's catalog, defined once: containers (`:::name`), typed blocks (fenced YAML) with their fields,
//! the check's rules and the kinds of folios. `folio_check` validates pages against it, `launchpad folio blocks` and
//! `launchpad folio rules` print it, and `gen-types` writes it for the UI's renderer (`web/src/gen/folioCatalog.ts`),
//! together with the data types of each block (ts-rs).
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The type of a block field.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Text (a number or a date is read as text).
    Text,
    /// An id: letters, digits, `-`, `_`, `.`, `:`; unique in the folio, marks hang on it.
    Id,
    Bool,
    /// One of these values.
    Choice(&'static [&'static str]),
    /// A list of texts.
    Texts,
    /// A list of ids of other items of the folio (`deps`).
    Ids,
    /// A list of items with these fields. `markable`: each item takes your marks, so it needs an id.
    Items { fields: &'static [Field], markable: bool },
    /// A list of rows, each a list of cells (`table`).
    Rows,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Field {
    pub name: &'static str,
    pub kind: Kind,
    pub required: bool,
    pub help: &'static str,
}

const fn req(name: &'static str, kind: Kind, help: &'static str) -> Field {
    Field { name, kind, required: true, help }
}
const fn opt(name: &'static str, kind: Kind, help: &'static str) -> Field {
    Field { name, kind, required: false, help }
}

/// A typed block: a fence whose language is its name, holding YAML.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct BlockDef {
    pub name: &'static str,
    pub help: &'static str,
    /// How the reader shows it.
    pub renders: &'static str,
    /// What marks it takes (empty: none).
    pub marks: &'static str,
    /// The block itself takes marks (a decision): its `id` is required.
    pub markable: bool,
    pub fields: &'static [Field],
    pub example: &'static str,
}

/// A layout container (`:::name{attrs} title`).
#[derive(Clone, Copy, Debug, Serialize)]
pub struct ContainerDef {
    pub name: &'static str,
    pub help: &'static str,
    /// Attributes it takes besides `#id` and `.class`, with their values (empty: any value).
    pub attrs: &'static [(&'static str, &'static [&'static str])],
    pub example: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "FolioSeverity")]
pub enum Severity {
    Error,
    Warn,
    Info,
    /// Seen in the reader (phase 4).
    Render,
}

impl Severity {
    pub fn name(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warn => "warn",
            Severity::Info => "info",
            Severity::Render => "render",
        }
    }
}

/// A rule of the check, with a bad and a good example.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Rule {
    pub code: &'static str,
    pub severity: Severity,
    pub title: &'static str,
    pub fix: &'static str,
    pub bad: &'static str,
    pub good: &'static str,
}

/// The kinds of folios: each picks a skeleton in `folio create` and the hints of rule F501.
pub const KINDS: &[(&str, &str)] = &[
    ("report", "Findings and a recommendation: verdict, decisions, details, sources"),
    ("roadmap", "Waves of work with items to decide on"),
    ("audit", "Findings with a severity, each to mark Do, Later, Won't do or Discuss"),
    ("brief", "A short digest: what happened, what waits, what is next"),
    ("status", "A status board of areas, with a timeline"),
    ("sketchbook", "Sketches and options to compare"),
];

pub const ADVICE: &[&str] = &["do", "later", "wont", "discuss"];
pub const SIZES: &[&str] = &["XS", "S", "M", "L", "XL"];
pub const TONES: &[&str] = &["verdict", "info", "note", "tip", "warn", "risk"];
pub const ALERTS: &[&str] = &["NOTE", "TIP", "IMPORTANT", "WARNING", "CAUTION"];

const ITEM_ROADMAP: &[Field] = &[
    req("id", Kind::Id, "Unique id, such as F-01: your verdict hangs on it"),
    req("title", Kind::Text, "One line"),
    opt("size", Kind::Choice(SIZES), "XS, S, M, L or XL"),
    opt("deps", Kind::Ids, "Ids of items it waits for"),
    opt("advice", Kind::Choice(ADVICE), "The author's advice: do, later, wont or discuss"),
    opt("status", Kind::Choice(&["todo", "doing", "done", "blocked"]), "todo, doing, done or blocked"),
    opt("text", Kind::Text, "A sentence or two (Markdown)"),
    opt("renamed_from", Kind::Texts, "Old ids of this item: your marks on them move here"),
];
const WAVES: &[Field] = &[
    req("id", Kind::Id, "Unique id, such as W1"),
    req("title", Kind::Text, "One line"),
    opt("size", Kind::Choice(SIZES), "XS, S, M, L or XL"),
    opt("text", Kind::Text, "A sentence or two (Markdown)"),
    opt("items", Kind::Items { fields: ITEM_ROADMAP, markable: true }, "The wave's items"),
    opt("renamed_from", Kind::Texts, "Old ids"),
];
const FINDINGS: &[Field] = &[
    req("id", Kind::Id, "Unique id, such as A-12"),
    req("title", Kind::Text, "One line"),
    opt("severity", Kind::Choice(&["critical", "high", "medium", "low", "info"]), "critical, high, medium, low or info"),
    opt("where", Kind::Text, "File, page or place"),
    opt("text", Kind::Text, "What and why (Markdown)"),
    opt("fix", Kind::Text, "The suggested fix"),
    opt("advice", Kind::Choice(ADVICE), "do, later, wont or discuss"),
    opt("renamed_from", Kind::Texts, "Old ids"),
];
const DECISION_OPTIONS: &[Field] = &[
    req("id", Kind::Id, "The option's id (your choice names it)"),
    req("label", Kind::Text, "A few words"),
    opt("text", Kind::Text, "What it means (Markdown)"),
    opt("recommended", Kind::Bool, "The author's recommendation (at most one)"),
];
const OPTION_ITEMS: &[Field] = &[
    req("id", Kind::Id, "The option's id"),
    req("label", Kind::Text, "A few words"),
    opt("text", Kind::Text, "Summary (Markdown)"),
    opt("pros", Kind::Texts, "Advantages"),
    opt("cons", Kind::Texts, "Drawbacks"),
    opt("cost", Kind::Text, "Cost or size"),
    opt("recommended", Kind::Bool, "The author's recommendation (at most one)"),
];
const DECISION_ROWS: &[Field] = &[
    opt("id", Kind::Id, "Optional id"),
    req("question", Kind::Text, "The question"),
    req("decision", Kind::Text, "What was decided"),
    opt("effect", Kind::Text, "Effect on the design"),
];
const TILES: &[Field] = &[
    opt("id", Kind::Id, "Optional id"),
    req("area", Kind::Text, "The area"),
    req("state", Kind::Choice(&["ok", "warn", "risk", "blocked", "done", "todo"]), "ok, warn, risk, blocked, done or todo"),
    opt("text", Kind::Text, "One line"),
    opt("link", Kind::Text, "A link"),
];
const TIMELINE: &[Field] = &[opt("id", Kind::Id, "Optional id"), req("date", Kind::Text, "A date (2026-11-18) or a label"), req("title", Kind::Text, "One line"), opt("text", Kind::Text, "Details (Markdown)")];
const STEPS: &[Field] = &[opt("id", Kind::Id, "Optional id"), req("title", Kind::Text, "One line"), opt("text", Kind::Text, "Details (Markdown)")];
const CHECKS: &[Field] = &[req("id", Kind::Id, "Unique id: your tick hangs on it"), req("text", Kind::Text, "One line"), opt("done", Kind::Bool, "Already done (the author's)")];
const METRICS: &[Field] = &[opt("id", Kind::Id, "Optional id"), req("label", Kind::Text, "What it measures"), req("value", Kind::Text, "The value, as text"), opt("text", Kind::Text, "One line"), opt("tone", Kind::Choice(&["good", "warn", "bad", "neutral"]), "good, warn, bad or neutral")];
const SOURCES: &[Field] = &[req("title", Kind::Text, "Title"), opt("url", Kind::Text, "Link"), opt("note", Kind::Text, "Why it matters")];

/// Every typed block.
pub const BLOCKS: &[BlockDef] = &[
    BlockDef { name: "roadmap", help: "Waves of work, each with items to mark Do, Later, Won't do or Discuss", renders: "Waves with their items, sizes and dependency hints", marks: "a verdict per item", markable: false,
        fields: &[req("waves", Kind::Items { fields: WAVES, markable: false }, "The waves, in order")],
        example: "waves:\n  - id: W1\n    title: Store, CLI and reader\n    size: M\n    items:\n      - { id: F-01, title: Plain store, advice: do }\n      - { id: F-02, title: Session tab, deps: [F-01] }\n" },
    BlockDef { name: "findings", help: "An audit list: one item per finding", renders: "Findings with severity, place and fix, filterable", marks: "a verdict and comments per item", markable: false,
        fields: &[req("items", Kind::Items { fields: FINDINGS, markable: true }, "The findings")],
        example: "items:\n  - id: A-01\n    title: Tokens are logged on retry\n    severity: high\n    where: crates/core/src/proxy.rs\n    fix: Redact before logging.\n    advice: do\n" },
    BlockDef { name: "decision", help: "One question with options to choose from", renders: "Option cards with the recommendation; your pick and a note", marks: "your choice and a note", markable: true,
        fields: &[req("id", Kind::Id, "Unique id, such as D-NAME"), req("question", Kind::Text, "The question"), opt("why", Kind::Text, "Why it matters (Markdown)"), req("options", Kind::Items { fields: DECISION_OPTIONS, markable: false }, "The options"), opt("renamed_from", Kind::Texts, "Old ids")],
        example: "id: D-NAME\nquestion: What do we call them?\noptions:\n  - { id: folio, label: Folio, recommended: true }\n  - { id: dossier, label: Dossier }\n" },
    BlockDef { name: "options", help: "Options compared side by side, with pros, cons and cost", renders: "A side-by-side comparison", marks: "your choice", markable: false,
        fields: &[opt("id", Kind::Id, "Id of the comparison (your choice hangs on it)"), opt("question", Kind::Text, "The question"), req("items", Kind::Items { fields: OPTION_ITEMS, markable: true }, "The options")],
        example: "id: O-STORE\nquestion: Where do folios live?\nitems:\n  - { id: plain, label: Plain store, pros: [backed up], cons: [new routes], recommended: true }\n  - { id: space, label: Static Space, pros: [public], cons: [cleanup] }\n" },
    BlockDef { name: "decisions", help: "The decisions recorded: question, decision, effect", renders: "A table of decisions", marks: "", markable: false,
        fields: &[req("rows", Kind::Items { fields: DECISION_ROWS, markable: false }, "One row per decision")],
        example: "rows:\n  - { question: Name, decision: Folio, effect: CLI launchpad folio }\n" },
    BlockDef { name: "status", help: "A status board", renders: "Tiles with state pills", marks: "", markable: false,
        fields: &[req("tiles", Kind::Items { fields: TILES, markable: false }, "One tile per area")],
        example: "tiles:\n  - { area: Store, state: done, text: Writes and revs work }\n  - { area: Reader, state: warn, text: Phone layout pending }\n" },
    BlockDef { name: "timeline", help: "Dated events", renders: "A dated timeline", marks: "", markable: false,
        fields: &[req("items", Kind::Items { fields: TIMELINE, markable: false }, "Events, oldest first")],
        example: "items:\n  - { date: 2026-11-12, title: Spaces cleaned up }\n  - { date: 2026-11-18, title: Decisions recorded }\n" },
    BlockDef { name: "steps", help: "Numbered steps", renders: "Numbered steps", marks: "", markable: false,
        fields: &[req("items", Kind::Items { fields: STEPS, markable: false }, "The steps, in order")],
        example: "items:\n  - { title: Write, text: The agent runs folio create. }\n  - { title: Read }\n" },
    BlockDef { name: "checklist", help: "Things to tick", renders: "Tick boxes", marks: "your ticks", markable: false,
        fields: &[opt("id", Kind::Id, "Optional id"), req("items", Kind::Items { fields: CHECKS, markable: true }, "The items")],
        example: "items:\n  - { id: C-1, text: Read the plan }\n  - { id: C-2, text: Pick a name, done: true }\n" },
    BlockDef { name: "metrics", help: "Key figures", renders: "KPI tiles", marks: "", markable: false,
        fields: &[req("items", Kind::Items { fields: METRICS, markable: false }, "2 to 6 figures")],
        example: "items:\n  - { label: Spaces built, value: \"35\", text: Oct 4 to Nov 12 }\n  - { label: Text ratio, value: 39 %, tone: warn }\n" },
    BlockDef { name: "table", help: "A table from rows, or from a CSV file of the folio", renders: "A sortable table", marks: "", markable: false,
        fields: &[opt("caption", Kind::Text, "Caption"), opt("columns", Kind::Texts, "Column names (with src: the CSV columns to show)"), opt("rows", Kind::Rows, "Rows of cells"), opt("src", Kind::Text, "A CSV file of the folio, such as data/usage.csv")],
        example: "caption: Sizes\ncolumns: [Space, KB]\nrows:\n  - [oracle report, 171]\n  - [audit, 113]\n" },
    BlockDef { name: "prompt", help: "A prompt for an implementer, with Copy", renders: "A prompt card with Copy", marks: "", markable: false,
        fields: &[opt("title", Kind::Text, "Title"), req("text", Kind::Text, "The prompt"), opt("profile", Kind::Text, "A profile hint"), opt("folder", Kind::Text, "A folder hint")],
        example: "title: Prompt for the phase 1 implementer\ntext: |\n  Implement phase 1 of Folios…\n" },
    BlockDef { name: "sources", help: "References", renders: "A reference list", marks: "", markable: false,
        fields: &[req("items", Kind::Items { fields: SOURCES, markable: false }, "The sources")],
        example: "items:\n  - { title: markdown-it, url: \"https://github.com/markdown-it/markdown-it\" }\n" },
];

/// Every container.
pub const CONTAINERS: &[ContainerDef] = &[
    ContainerDef { name: "callout", help: "An accent box; `> [!NOTE]` alerts become callouts too. Takes comments.", attrs: &[("tone", TONES)], example: ":::callout{tone=verdict} Verdict\nShip the store and reader first.\n:::\n" },
    ContainerDef { name: "cards", help: "A grid of cards: each `###` heading inside starts a card", attrs: &[], example: ":::cards\n### Content\nAgents write Markdown.\n### Feedback\nYou mark it on the page.\n:::\n" },
    ContainerDef { name: "columns", help: "Columns side by side: each `###` heading inside starts a column", attrs: &[], example: ":::columns\n### Before\nA static Space.\n### After\nA folio.\n:::\n" },
    ContainerDef { name: "tabs", help: "Tabs: each `###` heading inside starts a tab, named by the heading", attrs: &[], example: ":::tabs\n### Laptop\n…\n### Phone\n…\n:::\n" },
    ContainerDef { name: "details", help: "A folded section; its title is the summary (`{open}` starts it open)", attrs: &[("open", &[])], example: ":::details Method\nHow it was measured.\n:::\n" },
    ContainerDef { name: "scope", help: "In scope and out of scope: `### In` and `### Out` headings inside. Takes comments.", attrs: &[], example: ":::scope\n### In\n- Store\n### Out\n- Sharing\n:::\n" },
    ContainerDef { name: "hero", help: "A large lede under the title", attrs: &[], example: ":::hero\nAgents write content; Launchpad renders it.\n:::\n" },
];

const R: Rule = Rule { code: "", severity: Severity::Warn, title: "", fix: "", bad: "", good: "" };

/// Every rule of the check. Codes are stable.
pub const RULES: &[Rule] = &[
    Rule { code: "F101", severity: Severity::Error, title: "Front matter is not valid YAML, is never closed, or index.md has no title", bad: "---\ntitle: [x\n---", good: "---\ntitle: Roadmap\nkind: roadmap\n---", ..R },
    Rule { code: "F102", severity: Severity::Error, title: "A ::: container or a code fence is never closed: the rest of the page would be swallowed", bad: ":::callout\nText", good: ":::callout\nText\n:::", ..R },
    Rule { code: "F103", severity: Severity::Warn, title: "Unknown container name: it renders as a plain box", fix: "renames it when one name matches", bad: ":::warning", good: ":::callout{tone=warn}" },
    Rule { code: "F104", severity: Severity::Warn, title: "Unknown attribute or value in {…}", fix: "when one value matches", bad: ":::callout{tone=verdikt}", good: ":::callout{tone=verdict}" },
    Rule { code: "F105", severity: Severity::Warn, title: "pages: names a missing file, or a page is not listed (it would be unreachable)", bad: "pages: [index.md, plan.md]   # plan.md missing", good: "pages: [index.md, research.md]", ..R },
    Rule { code: "F106", severity: Severity::Warn, title: "Two headings get the same anchor id on one page: comments could attach to the wrong one", fix: "add {#id}", bad: "## Risks\n…\n## Risks", good: "## Risks {#risks-store}\n…\n## Risks {#risks-ui}" },
    Rule { code: "F107", severity: Severity::Warn, title: "Unknown GitHub alert type", fix: "when one type matches", bad: "> [!NOTICE]", good: "> [!NOTE]" },
    Rule { code: "F201", severity: Severity::Error, title: "The block's YAML does not parse", bad: "```decision\nid: D-1\nquestion: [x\n```", good: "```decision\nid: D-1\nquestion: Which one?\n```", ..R },
    Rule { code: "F202", severity: Severity::Error, title: "A required field is missing", bad: "```decision\nid: D-1\nquestion: Which?\n```", good: "```decision\nid: D-1\nquestion: Which?\noptions:\n  - { id: a, label: A }\n```", ..R },
    Rule { code: "F203", severity: Severity::Warn, title: "Unknown field: it is ignored", fix: "renames it when one field matches", bad: "advise: do", good: "advice: do" },
    Rule { code: "F204", severity: Severity::Error, title: "Wrong type or value (size: huge, a list given as text)", bad: "size: huge", good: "size: XL", ..R },
    Rule { code: "F205", severity: Severity::Error, title: "Duplicate id in the folio (items, decisions, explicit anchors): marks need unique ids", bad: "- { id: F-07 }\n- { id: F-07 }", good: "- { id: F-07 }\n- { id: F-08 }", ..R },
    Rule { code: "F206", severity: Severity::Warn, title: "An id your marks use was removed or renamed: those marks are orphaned. Use renamed_from: to keep them", bad: "(F-03 removed while you marked it)", good: "- { id: F-09, title: …, renamed_from: [F-03] }", ..R },
    Rule { code: "F207", severity: Severity::Warn, title: "Typed item without an id: it renders, but you cannot mark it", bad: "- { title: Plain store }", good: "- { id: F-01, title: Plain store }", ..R },
    Rule { code: "F208", severity: Severity::Warn, title: "Unknown block type: it renders as a key and value card", bad: "```x-poll", good: "```decision", ..R },
    Rule { code: "F209", severity: Severity::Warn, title: "A reference inside a block points nowhere (deps), or an option is recommended twice", bad: "deps: [W4]", good: "deps: [F-01]", ..R },
    Rule { code: "F301", severity: Severity::Error, title: "Link to a missing page, anchor or folio", bad: "[plan](plann.md)", good: "[plan](plan.md#waves)", ..R },
    Rule { code: "F302", severity: Severity::Error, title: "Missing image or data file", bad: "![flow](assets/flow.png)   # never added", good: "launchpad folio add-media <slug> assets/flow.png", ..R },
    Rule { code: "F303", severity: Severity::Warn, title: "Image from the network: the reader does not load it. Add it with add-media", bad: "![x](https://example.com/x.png)", good: "![x](assets/x.png)", ..R },
    Rule { code: "F304", severity: Severity::Warn, title: "A table names a column its CSV does not have", bad: "src: data/usage.csv\ncolumns: [day, cost]", good: "columns: [day, tokens]", ..R },
    Rule { code: "F501", severity: Severity::Info, title: "The kind's usual parts are missing (a roadmap without a verdict, an audit whose findings have no severity)", bad: "kind: roadmap   # no verdict", good: ":::callout{tone=verdict}", ..R },
    Rule { code: "F502", severity: Severity::Info, title: "Heading levels jump (## then ####): the contents list looks odd", bad: "## A\n#### B", good: "## A\n### B", ..R },
    Rule { code: "F601", severity: Severity::Warn, title: "Page over 60 KB: split it into pages, which also makes reads cheaper", bad: "one 90 KB page", good: "pages: [index.md, details.md]", ..R },
    Rule { code: "F602", severity: Severity::Warn, title: "Table with more than 7 columns, or a code line over 120 characters: it scrolls sideways on the phone", bad: "| a | b | c | d | e | f | g | h |", good: "two tables, or a findings block", ..R },
];

pub fn block(name: &str) -> Option<&'static BlockDef> {
    BLOCKS.iter().find(|b| b.name == name)
}

pub fn container(name: &str) -> Option<&'static ContainerDef> {
    CONTAINERS.iter().find(|c| c.name == name)
}

pub fn rule(code: &str) -> Option<&'static Rule> {
    RULES.iter().find(|r| r.code.eq_ignore_ascii_case(code))
}

/// Levenshtein distance.
pub fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            cur.push((prev[j] + usize::from(ca != cb)).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// The one name close to `word` (case-insensitive distance at most 2, or a prefix), None when none or several match.
pub fn closest<'a>(word: &str, names: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let w = word.to_lowercase();
    let hits: Vec<&str> = names.into_iter().filter(|n| {
        let n = n.to_lowercase();
        n != w && (distance(&n, &w) <= 2.min(w.len().saturating_sub(1)).max(1) || (w.len() >= 4 && (n.starts_with(&w) || w.starts_with(&n))))
    }).collect();
    (hits.len() == 1).then(|| hits[0])
}

/// The catalog as JSON, for the UI (`web/src/gen/folioCatalog.ts`) and `launchpad folio blocks --json`.
pub fn json() -> serde_json::Value {
    serde_json::json!({"blocks": BLOCKS, "containers": CONTAINERS, "rules": RULES, "kinds": KINDS.iter().map(|(k, h)| serde_json::json!({"name": k, "help": h})).collect::<Vec<_>>(), "alerts": ALERTS})
}

pub fn typescript() -> String {
    format!(
        "// Generated by `launchpad gen-types` from crates/automation/src/folio_catalog.rs. Do not edit.\n\nexport const FOLIO_CATALOG = {} as const;\n",
        serde_json::to_string_pretty(&json()).expect("serializable")
    )
}

/// Text of one block for `launchpad folio blocks <name>`.
pub fn block_text(b: &BlockDef) -> String {
    let mut out = format!("```{}  {}\n  renders as: {}\n", b.name, b.help, b.renders);
    if !b.marks.is_empty() {
        out.push_str(&format!("  your marks: {}\n", b.marks));
    }
    out.push_str("  fields:\n");
    fn fields(out: &mut String, fs: &[Field], indent: usize) {
        for f in fs {
            let kind = match f.kind {
                Kind::Text => "text".to_string(),
                Kind::Id => "id".to_string(),
                Kind::Bool => "true|false".to_string(),
                Kind::Choice(v) => v.join("|"),
                Kind::Texts => "[text]".to_string(),
                Kind::Ids => "[id]".to_string(),
                Kind::Items { markable, .. } => if markable { "list of items (each needs an id)".into() } else { "list of items".into() },
                Kind::Rows => "[[cell]]".to_string(),
            };
            out.push_str(&format!("{}{}{}: {kind} · {}\n", " ".repeat(indent), f.name, if f.required { "" } else { "?" }, f.help));
            if let Kind::Items { fields: sub, .. } = f.kind {
                fields(out, sub, indent + 2);
            }
        }
    }
    fields(&mut out, b.fields, 4);
    out.push_str(&format!("  example:\n```{}\n{}```\n", b.name, b.example));
    out
}

pub fn container_text(c: &ContainerDef) -> String {
    let attrs = c.attrs.iter().map(|(k, v)| if v.is_empty() { k.to_string() } else { format!("{k}={}", v.join("|")) }).collect::<Vec<_>>().join(" ");
    format!(":::{}{}  {}\n{}", c.name, if attrs.is_empty() { String::new() } else { format!("{{{attrs}}}") }, c.help, c.example)
}

pub fn rule_text(r: &Rule) -> String {
    let mut out = format!("{} {}  {}\n", r.code, r.severity.name(), r.title);
    if !r.fix.is_empty() {
        out.push_str(&format!("  --fix: {}\n", r.fix));
    }
    out.push_str(&format!("  bad:\n{}\n  good:\n{}\n", indent(r.bad), indent(r.good)));
    out
}

fn indent(s: &str) -> String {
    s.lines().map(|l| format!("    {l}")).collect::<Vec<_>>().join("\n")
}

// ---------------------------------------------------------------- the data of each block, for the UI (ts-rs)

macro_rules! data {
    ($name:ident, $ts:literal { $($field:ident : $ty:ty),* $(,)? }) => {
        #[derive(Clone, Debug, Deserialize, Serialize, TS)]
        #[serde(deny_unknown_fields)]
        #[ts(rename = $ts)]
        pub struct $name { $( #[ts(optional)] pub $field: Option<$ty>, )* }
    };
}

/// Text fields accept numbers too (YAML reads `42` as a number).
pub type Txt = serde_json::Value;

data!(RoadmapItem, "FolioRoadmapItem" { id: Txt, title: Txt, size: String, deps: Vec<Txt>, advice: String, status: String, text: Txt, renamed_from: Vec<Txt> });
data!(Wave, "FolioWave" { id: Txt, title: Txt, size: String, text: Txt, items: Vec<RoadmapItem>, renamed_from: Vec<Txt> });
data!(RoadmapBlock, "FolioRoadmap" { waves: Vec<Wave> });
data!(Finding, "FolioFinding" { id: Txt, title: Txt, severity: String, r#where: Txt, text: Txt, fix: Txt, advice: String, renamed_from: Vec<Txt> });
data!(FindingsBlock, "FolioFindings" { items: Vec<Finding> });
data!(DecisionOption, "FolioDecisionOption" { id: Txt, label: Txt, text: Txt, recommended: bool });
data!(DecisionBlock, "FolioDecision" { id: Txt, question: Txt, why: Txt, options: Vec<DecisionOption>, renamed_from: Vec<Txt> });
data!(OptionItem, "FolioOption" { id: Txt, label: Txt, text: Txt, pros: Vec<Txt>, cons: Vec<Txt>, cost: Txt, recommended: bool });
data!(OptionsBlock, "FolioOptions" { id: Txt, question: Txt, items: Vec<OptionItem> });
data!(DecisionRow, "FolioDecisionRow" { id: Txt, question: Txt, decision: Txt, effect: Txt });
data!(DecisionsBlock, "FolioDecisions" { rows: Vec<DecisionRow> });
data!(StatusTile, "FolioStatusTile" { id: Txt, area: Txt, state: String, text: Txt, link: Txt });
data!(StatusBlock, "FolioStatus" { tiles: Vec<StatusTile> });
data!(TimelineItem, "FolioTimelineItem" { id: Txt, date: Txt, title: Txt, text: Txt });
data!(TimelineBlock, "FolioTimeline" { items: Vec<TimelineItem> });
data!(Step, "FolioStep" { id: Txt, title: Txt, text: Txt });
data!(StepsBlock, "FolioSteps" { items: Vec<Step> });
data!(CheckItem, "FolioCheckItem" { id: Txt, text: Txt, done: bool });
data!(ChecklistBlock, "FolioChecklist" { id: Txt, items: Vec<CheckItem> });
data!(Metric, "FolioMetric" { id: Txt, label: Txt, value: Txt, text: Txt, tone: String });
data!(MetricsBlock, "FolioMetrics" { items: Vec<Metric> });
data!(TableBlock, "FolioTable" { caption: Txt, columns: Vec<Txt>, rows: Vec<Vec<Txt>>, src: String });
data!(PromptBlock, "FolioPrompt" { title: Txt, text: Txt, profile: Txt, folder: Txt });
data!(Source, "FolioSource" { title: Txt, url: Txt, note: Txt });
data!(SourcesBlock, "FolioSources" { items: Vec<Source> });

/// Exports the block data types.
pub fn export(cfg: &ts_rs::Config) -> anyhow::Result<()> {
    RoadmapBlock::export_all(cfg)?;
    FindingsBlock::export_all(cfg)?;
    DecisionBlock::export_all(cfg)?;
    OptionsBlock::export_all(cfg)?;
    DecisionsBlock::export_all(cfg)?;
    StatusBlock::export_all(cfg)?;
    TimelineBlock::export_all(cfg)?;
    StepsBlock::export_all(cfg)?;
    ChecklistBlock::export_all(cfg)?;
    MetricsBlock::export_all(cfg)?;
    TableBlock::export_all(cfg)?;
    PromptBlock::export_all(cfg)?;
    SourcesBlock::export_all(cfg)?;
    std::fs::write(cfg.out_dir().join("folioCatalog.ts"), typescript())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(name: &str) -> String {
        let cfg = ts_rs::Config::default();
        match name {
            "roadmap" => format!("{}{}{}", RoadmapBlock::decl(&cfg), Wave::decl(&cfg), RoadmapItem::decl(&cfg)),
            "findings" => format!("{}{}", FindingsBlock::decl(&cfg), Finding::decl(&cfg)),
            "decision" => format!("{}{}", DecisionBlock::decl(&cfg), DecisionOption::decl(&cfg)),
            "options" => format!("{}{}", OptionsBlock::decl(&cfg), OptionItem::decl(&cfg)),
            "decisions" => format!("{}{}", DecisionsBlock::decl(&cfg), DecisionRow::decl(&cfg)),
            "status" => format!("{}{}", StatusBlock::decl(&cfg), StatusTile::decl(&cfg)),
            "timeline" => format!("{}{}", TimelineBlock::decl(&cfg), TimelineItem::decl(&cfg)),
            "steps" => format!("{}{}", StepsBlock::decl(&cfg), Step::decl(&cfg)),
            "checklist" => format!("{}{}", ChecklistBlock::decl(&cfg), CheckItem::decl(&cfg)),
            "metrics" => format!("{}{}", MetricsBlock::decl(&cfg), Metric::decl(&cfg)),
            "table" => TableBlock::decl(&cfg),
            "prompt" => PromptBlock::decl(&cfg),
            "sources" => format!("{}{}", SourcesBlock::decl(&cfg), Source::decl(&cfg)),
            other => panic!("no data type for {other}"),
        }
    }

    fn parses(name: &str, example: &str) -> bool {
        macro_rules! p { ($t:ty) => { serde_yaml_ng::from_str::<$t>(example).is_ok() } }
        match name {
            "roadmap" => p!(RoadmapBlock),
            "findings" => p!(FindingsBlock),
            "decision" => p!(DecisionBlock),
            "options" => p!(OptionsBlock),
            "decisions" => p!(DecisionsBlock),
            "status" => p!(StatusBlock),
            "timeline" => p!(TimelineBlock),
            "steps" => p!(StepsBlock),
            "checklist" => p!(ChecklistBlock),
            "metrics" => p!(MetricsBlock),
            "table" => p!(TableBlock),
            "prompt" => p!(PromptBlock),
            "sources" => p!(SourcesBlock),
            _ => false,
        }
    }

    /// The catalog and the data types the UI renders stay one definition: every field of a block (and of its items)
    /// is a field of its type, and every example parses into it.
    #[test]
    fn catalog_matches_the_data_types() {
        fn names(fs: &[Field], out: &mut Vec<&'static str>) {
            for f in fs {
                out.push(f.name);
                if let Kind::Items { fields, .. } = f.kind {
                    names(fields, out);
                }
            }
        }
        for b in BLOCKS {
            let d = decl(b.name);
            let mut ns = vec![];
            names(b.fields, &mut ns);
            for n in ns {
                assert!(d.contains(&format!("{n}?:")), "{}: field {n} missing from its type: {d}", b.name);
            }
            assert!(parses(b.name, b.example), "{}: example does not parse", b.name);
        }
    }

    #[test]
    fn suggestions() {
        assert_eq!(closest("advise", ["advice", "status", "size"]), Some("advice"));
        assert_eq!(closest("warning", CONTAINERS.iter().map(|c| c.name)), None);
        assert_eq!(closest("calout", CONTAINERS.iter().map(|c| c.name)), Some("callout"));
        assert_eq!(closest("verdikt", TONES.iter().copied()), Some("verdict"));
        assert_eq!(closest("NOTICE", ALERTS.iter().copied()), Some("NOTE"));
    }
}

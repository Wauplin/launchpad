//! Folios: reports, plans and sketchbooks that agents write as Markdown with typed blocks, and that Launchpad
//! renders, versions and hosts on Home.
//!
//! - A folio is a record (`folio`: workspace, slug, title, kind, state, author, rev, topic, thread) and a plain store
//!   `folio-<id>` (`plain::folio_store`) that only Home writes, through `write` (one writer at a time). Each write is
//!   a rev (`folio_rev`: the files it changed, before and after, its author and its problem counts), with diff and
//!   Restore. A write names the rev it started from (`base_rev`): a file changed since keeps both versions (the new
//!   one as `<name>.concurrent-<id>.md`), like the knowledge base.
//! - Every write is checked (`folio_check`): the problems come back in the answer, the write is saved anyway unless
//!   it asked for `strict` (then a write with errors is refused). Hard limits refuse it: a path outside the folio,
//!   a file over 2 MB, a page that is not UTF-8.
//! - The sessions that write a folio join its topic and get a row in `folio_link` (role created or edited); a bot's
//!   session also makes its thread the folio's thread (when it has none), where Send feedback goes.
//! - Marks (`folio_mark`) are the owner's comments, choices, verdicts and ticks (saved at once, unsent until Send
//!   feedback) and the agents' replies; agents resolve comments naming the rev that addressed them. `seq` orders
//!   every change, so `feedback --since <seq>` lists only what is new.
//! - A folio with no change and no open mark for 30 days archives itself; Bring back restores it as it was.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail, ensure};
use launchpad_core::App;
use launchpad_core::kb::{Author, AuthorKind};
use launchpad_core::plain;
use launchpad_core::util::now_ms;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

use crate::folio_catalog::{self as cat, Severity};
use crate::folio_check::{self as check, Problem};
use crate::folio_split as split;

/// Largest file of a folio.
pub const MAX_FILE: usize = 2 << 20;
/// A folio archives itself after this long without a change or an open mark.
pub const ARCHIVE_AFTER_MS: i64 = 30 * 24 * 3600 * 1000;

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct FolioThread {
    pub bot_id: String,
    pub thread_id: String,
}

/// A session that worked on a folio.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct FolioLink {
    pub session_id: String,
    /// created, edited or replied.
    pub role: String,
    #[ts(type = "number")]
    pub first_rev: i64,
    #[ts(type = "number")]
    pub last_rev: i64,
    #[ts(type = "number")]
    pub at: i64,
    /// The session's title, when Home still has it.
    pub title: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Folio {
    pub id: String,
    pub workspace_id: String,
    pub slug: String,
    pub title: String,
    pub kind: String,
    /// active or archived.
    pub state: String,
    pub created_by: Author,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
    #[ts(type = "number")]
    pub rev: i64,
    pub topic_id: Option<String>,
    /// The bot thread Send feedback goes to.
    pub thread: Option<FolioThread>,
    #[ts(type = "number | null")]
    pub archived_at: Option<i64>,
    /// Problems of the latest rev.
    pub errors: u32,
    pub warnings: u32,
    /// From index.md's front matter.
    pub status: Option<String>,
    pub summary: Option<String>,
    /// Your marks not sent yet, and comments not resolved.
    pub unsent: u32,
    pub open: u32,
    pub links: Vec<FolioLink>,
}

/// A page of a folio with its outline.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct FolioPage {
    pub path: String,
    /// The front matter's title, else the first heading, else the file name.
    pub title: String,
    pub text: String,
    pub outline: split::Outline,
}

/// A folio as the reader shows it (at its latest rev, or at `rev`).
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct FolioDetail {
    pub folio: Folio,
    #[ts(type = "number")]
    pub rev: i64,
    pub pages: Vec<FolioPage>,
    /// Every file (pages, assets, data).
    pub files: Vec<plain::Entry>,
    pub problems: Vec<Problem>,
    pub marks: Vec<Mark>,
}

/// One rev: a write, with the files it changed.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(rename = "FolioRev")]
pub struct Rev {
    #[ts(type = "number")]
    pub rev: i64,
    #[ts(type = "number")]
    pub at: i64,
    pub author: Author,
    pub session_id: Option<String>,
    pub label: Option<String>,
    pub files: Vec<plain::Written>,
    pub errors: u32,
    pub warnings: u32,
}

/// A write's change, recorded with its folio (receipts in chats and traces).
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(rename = "FolioActivity")]
pub struct Activity {
    pub folio_id: String,
    pub slug: String,
    pub title: String,
    #[ts(type = "number")]
    pub rev: i64,
    #[ts(type = "number")]
    pub at: i64,
    pub session_id: Option<String>,
    pub paths: Vec<String>,
    pub created: bool,
}

/// A mark: the owner's comment, choice, verdict or tick, or an agent's reply.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(rename = "FolioMark")]
pub struct Mark {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub seq: i64,
    /// comment, choice, verdict, check or reply.
    pub kind: String,
    pub author: Author,
    pub page: Option<String>,
    /// The block, item or heading id it hangs on.
    pub target: Option<String>,
    /// A comment's quoted text, and about 30 characters around it.
    pub quote: Option<String>,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    /// verdict: do, later, wont or discuss; choice: the option id; check: true or false.
    pub value: Option<String>,
    pub text: Option<String>,
    /// A reply's comment.
    #[ts(type = "number | null")]
    pub parent: Option<i64>,
    /// The rev it was made on.
    #[ts(type = "number")]
    pub rev: i64,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
    #[ts(type = "number | null")]
    pub sent_at: Option<i64>,
    #[ts(type = "number | null")]
    pub resolved_at: Option<i64>,
    #[ts(type = "number | null")]
    pub resolved_rev: Option<i64>,
}

/// What a write answers.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(rename = "FolioWritten")]
pub struct Written {
    pub folio: Folio,
    /// False: refused (`strict` and errors), or nothing changed.
    pub written: bool,
    #[ts(type = "number")]
    pub rev: i64,
    pub lines: Vec<String>,
    pub problems: Vec<Problem>,
}

/// One write of `write`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    Write { path: String, data: Vec<u8> },
    Append { path: String, text: String },
    /// Replaces a section (a heading to the next of its level, a container or a typed block, by id).
    Patch { path: String, section: String, text: String },
    Remove { path: String },
}

// ---------------------------------------------------------------- pure helpers

/// A slug from a title: lowercase letters and digits joined by `-`, at most 60 characters.
pub fn slugify(title: &str) -> String {
    let mut out = String::new();
    for c in title.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let mut s: String = out.trim_matches('-').chars().take(60).collect();
    s = s.trim_end_matches('-').to_string();
    if s.is_empty() { "folio".into() } else { s }
}

pub fn valid_slug(slug: &str) -> bool {
    let b = slug.as_bytes();
    !b.is_empty() && b.len() <= 63 && b[0].is_ascii_alphanumeric() && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// A folio path: relative, no `..`, no hidden or Git parts.
pub fn check_path(path: &str) -> Result<()> {
    ensure!(!path.is_empty() && path.len() <= 200, "a folio path is 1 to 200 characters");
    ensure!(!path.starts_with('/') && !path.contains('\\') && path.split('/').all(|c| !c.is_empty() && c != "." && c != ".." && !c.starts_with('.')), "{path:?} is outside the folio (use a relative path such as plan.md or assets/flow.png)");
    ensure!(!path.chars().any(char::is_control), "{path:?} holds a control character");
    Ok(())
}

/// `name.concurrent-<id>.ext`.
fn concurrent_path(path: &str, id: &str) -> String {
    let (dir, file) = path.rsplit_once('/').map(|(d, f)| (format!("{d}/"), f)).unwrap_or((String::new(), path));
    match file.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{dir}{stem}.concurrent-{id}.{ext}"),
        _ => format!("{dir}{file}.concurrent-{id}"),
    }
}

/// A page's title: the front matter's, else its first heading, else the file name.
pub fn page_title(path: &str, text: &str) -> String {
    if let Some(m) = check::front(text)
        && let Some(t) = m.get(serde_yaml_ng::Value::String("title".into())).and_then(|v| v.as_str())
        && !t.trim().is_empty()
    {
        return t.trim().to_string();
    }
    for i in split::outline(text).items {
        if let split::Item::Heading { text, .. } = i {
            return text;
        }
    }
    path.trim_end_matches(".md").rsplit('/').next().unwrap_or(path).to_string()
}

/// The message Send feedback sends: one line that counts the marks, and the command that reads them.
pub fn feedback_message(title: &str, slug: &str, rev: i64, marks: &[Mark], since: i64) -> String {
    let n = |k: &str| marks.iter().filter(|m| m.kind == k).count();
    let mut parts = vec![];
    for (k, one, many) in [("comment", "comment", "comments"), ("verdict", "verdict", "verdicts"), ("choice", "decision", "decisions"), ("check", "tick", "ticks")] {
        match n(k) {
            0 => {}
            1 => parts.push(format!("1 {one}")),
            c => parts.push(format!("{c} {many}")),
        }
    }
    format!("Feedback on the folio “{title}” (rev {rev}): {}. Read it with `launchpad folio feedback {slug} --since {since}`, then reply to each comment and resolve it with the rev that addressed it (`launchpad folio reply|resolve`). The folio: /folios/{slug}", if parts.is_empty() { "no marks".into() } else { parts.join(", ") })
}

/// Your marks as the few lines `launchpad folio feedback` prints.
pub fn feedback_text(slug: &str, rev: i64, marks: &[Mark], replies: &[Mark]) -> String {
    let mut out = vec![];
    let sent = marks.iter().filter_map(|m| m.sent_at).max();
    out.push(format!("folio {slug} · rev {rev} · {} mark{} from the owner{}", marks.len(), if marks.len() == 1 { "" } else { "s" }, sent.map(|_| " (sent)").unwrap_or_default()));
    for m in marks.iter().filter(|m| m.kind == "comment") {
        let place = [m.page.clone(), m.target.as_ref().map(|t| format!("#{t}"))].into_iter().flatten().collect::<Vec<_>>().join(" › ");
        let quote = m.quote.as_deref().filter(|q| !q.is_empty()).map(|q| format!("  \"{}\"", short(q, 60))).unwrap_or_default();
        let state = if m.resolved_at.is_some() { format!(" (resolved in rev {})", m.resolved_rev.unwrap_or(0)) } else { String::new() };
        out.push(format!("comment c-{}  {place}{quote}{state}", m.id));
        for l in m.text.as_deref().unwrap_or("").lines() {
            out.push(format!("  {l}"));
        }
        for r in replies.iter().filter(|r| r.parent == Some(m.id)) {
            out.push(format!("  ↳ {}: {}", r.author.name, short(r.text.as_deref().unwrap_or(""), 100)));
        }
    }
    let list = |k: &str| marks.iter().filter(|m| m.kind == k).map(|m| format!("{} {}", m.target.as_deref().unwrap_or("?"), m.value.as_deref().unwrap_or(""))).collect::<Vec<_>>();
    let v = list("verdict");
    if !v.is_empty() {
        out.push(format!("verdict  {}", v.join(" · ")));
    }
    for m in marks.iter().filter(|m| m.kind == "choice") {
        let note = m.text.as_deref().filter(|t| !t.is_empty()).map(|t| format!("  note: {t}")).unwrap_or_default();
        out.push(format!("choice   {} → {}{note}", m.target.as_deref().unwrap_or("?"), m.value.as_deref().unwrap_or("")));
    }
    let c = list("check");
    if !c.is_empty() {
        out.push(format!("check    {}", c.join(" · ")));
    }
    if let Some(first) = marks.iter().find(|m| m.target.is_some()) {
        out.push(format!("next: launchpad folio read {slug} {} --section {}", first.page.as_deref().unwrap_or("index.md"), first.target.as_deref().unwrap_or("")));
    }
    out.join("\n")
}

fn short(s: &str, n: usize) -> String {
    let one = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= n { one } else { format!("{}…", one.chars().take(n - 1).collect::<String>()) }
}

// ---------------------------------------------------------------- records

fn ensure_home(app: &App) -> Result<()> {
    ensure!(launchpad_core::home_git::here(app), "folios are stored by Home");
    Ok(())
}

static WRITES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const COLUMNS: &str = "id, workspace_id, slug, title, kind, state, created_by, created_at, updated_at, rev, topic_id, thread_bot, thread_id, archived_at, problems";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Folio> {
    let problems: Vec<u32> = serde_json::from_str(&r.get::<_, String>(14)?).unwrap_or_default();
    let (bot, thread): (Option<String>, Option<String>) = (r.get(11)?, r.get(12)?);
    Ok(Folio {
        id: r.get(0)?,
        workspace_id: r.get(1)?,
        slug: r.get(2)?,
        title: r.get(3)?,
        kind: r.get(4)?,
        state: r.get(5)?,
        created_by: serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_else(|_| Author::owner(None)),
        created_at: r.get(7)?,
        updated_at: r.get(8)?,
        rev: r.get(9)?,
        topic_id: r.get(10)?,
        thread: bot.zip(thread).map(|(bot_id, thread_id)| FolioThread { bot_id, thread_id }),
        archived_at: r.get(13)?,
        errors: problems.first().copied().unwrap_or(0),
        warnings: problems.get(1).copied().unwrap_or(0),
        status: None,
        summary: None,
        unsent: 0,
        open: 0,
        links: vec![],
    })
}

type LinkRow = (String, String, i64, i64, i64);

async fn fill(app: &App, mut f: Folio) -> Result<Folio> {
    let id = f.id.clone();
    let (unsent, open, links): (u32, u32, Vec<LinkRow>) = app.state_db.call(move |c| {
        let unsent = c.query_row("SELECT count(*) FROM folio_mark WHERE folio_id = ?1 AND deleted = 0 AND kind != 'reply' AND session_id IS NULL AND sent_at IS NULL", [&id], |r| r.get(0))?;
        let open = c.query_row("SELECT count(*) FROM folio_mark WHERE folio_id = ?1 AND deleted = 0 AND kind = 'comment' AND resolved_at IS NULL", [&id], |r| r.get(0))?;
        let mut st = c.prepare("SELECT session_id, role, first_rev, last_rev, at FROM folio_link WHERE folio_id = ?1 ORDER BY at DESC")?;
        let links = st.query_map([&id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((unsent, open, links))
    }).await?;
    f.unsent = unsent;
    f.open = open;
    f.links = links.into_iter().map(|(session_id, role, first_rev, last_rev, at)| FolioLink { title: app.sessions.get(&session_id).map(|s| s.title), session_id, role, first_rev, last_rev, at }).collect();
    if let Ok(t) = file_text(app, &f.id, "index.md", None).await
        && let Some(m) = check::front(&t)
    {
        let g = |k: &str| m.get(serde_yaml_ng::Value::String(k.into())).and_then(|v| v.as_str()).map(str::to_string).filter(|s| !s.is_empty());
        f.status = g("status");
        f.summary = g("summary");
    }
    Ok(f)
}

/// The folios of a workspace (active, or archived with `archived`), newest change first.
pub async fn list(app: &App, ws: &str, archived: bool) -> Result<Vec<Folio>> {
    ensure_home(app)?;
    let ws = ws.to_string();
    let rows = app.state_db.call(move |c| {
        let mut st = c.prepare(&format!("SELECT {COLUMNS} FROM folio WHERE workspace_id = ?1 AND state = ?2 ORDER BY updated_at DESC"))?;
        let rows = st.query_map(params![ws, if archived { "archived" } else { "active" }], row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }).await?;
    let mut out = vec![];
    for f in rows {
        out.push(fill(app, f).await?);
    }
    Ok(out)
}

/// A folio of a workspace by slug (or id).
pub async fn get(app: &App, ws: &str, slug: &str) -> Result<Folio> {
    ensure_home(app)?;
    let (w, s) = (ws.to_string(), slug.to_string());
    let f = app.state_db.call(move |c| Ok(c.query_row(&format!("SELECT {COLUMNS} FROM folio WHERE workspace_id = ?1 AND (slug = ?2 OR id = ?2)"), params![w, s], row).optional()?)).await?;
    fill(app, f.with_context(|| format!("no folio {slug} in this workspace (launchpad folio list)"))?).await
}

/// The folios a session worked on (its Folio tab).
pub async fn of_session(app: &App, session: &str) -> Result<Vec<Folio>> {
    let s = session.to_string();
    let ids: Vec<(String, String)> = app.state_db.call(move |c| {
        let mut st = c.prepare("SELECT f.workspace_id, f.id FROM folio f JOIN folio_link l ON l.folio_id = f.id WHERE l.session_id = ?1 ORDER BY l.at DESC")?;
        let rows = st.query_map([s], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }).await?;
    let mut out = vec![];
    for (ws, id) in ids {
        if let Ok(f) = get(app, &ws, &id).await {
            out.push(f);
        }
    }
    Ok(out)
}

async fn tree(app: &App, id: &str) -> Result<plain::Tree> {
    plain::tree(app, &plain::folio_store(id)).await
}

/// The content of a file at the latest rev, or at `rev`.
pub async fn file_bytes(app: &App, id: &str, path: &str, rev: Option<i64>) -> Result<Vec<u8>> {
    let sha = match rev {
        None => tree(app, id).await?.files.into_iter().find(|f| f.path == path).map(|f| f.sha256),
        Some(r) => files_at(app, id, r).await?.remove(path),
    };
    let sha = sha.with_context(|| format!("there is no {path} in this folio{}", rev.map(|r| format!(" at rev {r}")).unwrap_or_default()))?;
    plain::read_blob(app, &sha).await
}

pub async fn file_text(app: &App, id: &str, path: &str, rev: Option<i64>) -> Result<String> {
    String::from_utf8(file_bytes(app, id, path, rev).await?).context("not a text file")
}

/// The files (path → content hash) as they were at a rev, rebuilt from the revs.
async fn files_at(app: &App, id: &str, rev: i64) -> Result<BTreeMap<String, String>> {
    let mut files = BTreeMap::new();
    for r in revs(app, id).await?.into_iter().rev().filter(|r| r.rev <= rev) {
        for w in r.files {
            match w.after {
                Some(h) => files.insert(w.path, h),
                None => files.remove(&w.path),
            };
        }
    }
    Ok(files)
}

/// Every text file (pages, CSV) and every path, at the latest rev or at `rev`.
async fn contents(app: &App, id: &str, rev: Option<i64>) -> Result<(BTreeMap<String, String>, Vec<plain::Entry>)> {
    let entries: Vec<plain::Entry> = match rev {
        None => tree(app, id).await?.files,
        Some(r) => files_at(app, id, r).await?.into_iter().map(|(path, sha256)| plain::Entry { path, sha256, size: 0, executable: false }).collect(),
    };
    let mut texts = BTreeMap::new();
    for e in &entries {
        if is_text(&e.path)
            && let Ok(d) = plain::read_blob(app, &e.sha256).await
            && let Ok(t) = String::from_utf8(d)
        {
            texts.insert(e.path.clone(), t);
        }
    }
    Ok((texts, entries))
}

fn is_text(path: &str) -> bool {
    [".md", ".csv", ".txt", ".json", ".yaml", ".yml", ".html", ".svg"].iter().any(|e| path.ends_with(e))
}

async fn slugs(app: &App, ws: &str) -> BTreeSet<String> {
    let ws = ws.to_string();
    app.state_db.call(move |c| {
        let mut st = c.prepare("SELECT slug FROM folio WHERE workspace_id = ?1")?;
        let rows = st.query_map([ws], |r| r.get(0))?.collect::<rusqlite::Result<BTreeSet<String>>>()?;
        Ok(rows)
    }).await.unwrap_or_default()
}

fn mark_refs(marks: &[Mark]) -> Vec<check::MarkRef> {
    marks.iter().filter(|m| m.kind != "reply" && m.author.kind == AuthorKind::Owner && m.resolved_at.is_none()).filter_map(|m| Some(check::MarkRef { id: m.id, kind: m.kind.clone(), target: m.target.clone()? })).collect()
}

/// The check of the stored folio (or of one rev).
pub async fn check_stored(app: &App, f: &Folio, rev: Option<i64>) -> Result<Vec<Problem>> {
    let (texts, entries) = contents(app, &f.id, rev).await?;
    let marks = marks(app, &f.id, None).await?;
    Ok(check::check(&check::Input { texts, paths: entries.into_iter().map(|e| e.path).collect(), marks: mark_refs(&marks), slugs: Some(slugs(app, &f.workspace_id).await) }))
}

/// The reader's view of a folio.
pub async fn detail(app: &App, ws: &str, slug: &str, rev: Option<i64>) -> Result<FolioDetail> {
    let folio = get(app, ws, slug).await?;
    let (texts, entries) = contents(app, &folio.id, rev).await?;
    let marks_list = marks(app, &folio.id, None).await?;
    let problems = check::check(&check::Input { texts: texts.clone(), paths: entries.iter().map(|e| e.path.clone()).collect(), marks: mark_refs(&marks_list), slugs: Some(slugs(app, ws).await) });
    let pages = check::page_order(&texts).into_iter().map(|path| {
        let text = texts.get(&path).cloned().unwrap_or_default();
        FolioPage { title: page_title(&path, &text), outline: split::outline(&text), path, text }
    }).collect();
    Ok(FolioDetail { rev: rev.unwrap_or(folio.rev), folio, pages, files: entries, problems, marks: marks_list })
}

/// Creates a folio from its kind's skeleton, as rev 1 by `author`.
pub async fn create(app: &App, ws: &str, title: &str, kind: &str, slug: Option<&str>, author: Author) -> Result<Written> {
    ensure_home(app)?;
    launchpad_core::workspace::get(app, ws).await?;
    let title = title.trim();
    ensure!(!title.is_empty() && title.chars().count() <= 120 && !title.contains('\n'), "a folio's title is one line of 1 to 120 characters");
    ensure!(cat::KINDS.iter().any(|(k, _)| *k == kind), "unknown kind {kind:?}: {}", cat::KINDS.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(", "));
    let base = match slug {
        Some(s) => {
            ensure!(valid_slug(s), "a slug uses lowercase letters, digits and - (at most 63)");
            s.to_string()
        }
        None => slugify(title),
    };
    let taken = slugs(app, ws).await;
    let slug = if slug.is_some() {
        ensure!(!taken.contains(&base), "this workspace already has a folio {base}: update it (launchpad folio write) or pick another --slug");
        base
    } else {
        (1..).map(|n| if n == 1 { base.clone() } else { format!("{base}-{n}") }).find(|s| !taken.contains(s)).expect("a free slug")
    };
    let id = uuid::Uuid::new_v4().to_string();
    let topic = topic_for(app, ws, title).await.ok();
    let (i, w, s, t, k, a, at, top) = (id.clone(), ws.to_string(), slug.clone(), title.to_string(), kind.to_string(), serde_json::to_string(&author)?, now_ms(), topic.clone());
    app.state_db.call(move |c| {
        c.execute("INSERT INTO folio (id, workspace_id, slug, title, kind, created_by, created_at, updated_at, topic_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8)", params![i, w, s, t, k, a, at, top])?;
        Ok(())
    }).await?;
    let skeleton = check::skeleton(title, kind);
    write(app, ws, &slug, vec![Op::Write { path: "index.md".into(), data: skeleton.into_bytes() }], author, None, false, Some("created")).await
}

/// A topic for a new folio: its title, else "Folio · title", else with its slug.
async fn topic_for(app: &App, ws: &str, title: &str) -> Result<String> {
    let name: String = title.chars().take(70).collect();
    for n in [name.clone(), format!("Folio · {name}"), format!("Folio · {name} ({})", &uuid::Uuid::new_v4().simple().to_string()[..4])] {
        if let Ok(t) = launchpad_core::topics::create(app, ws, &n).await {
            return Ok(t.id);
        }
    }
    bail!("no topic")
}

/// Writes to a folio, one writer at a time. `base_rev`: the rev the writer read (a file changed since keeps both
/// versions). `strict`: refuse the write when it has errors. Returns the problems of the folio after the write.
#[allow(clippy::too_many_arguments)]
pub async fn write(app: &App, ws: &str, slug: &str, ops: Vec<Op>, author: Author, base_rev: Option<i64>, strict: bool, label: Option<&str>) -> Result<Written> {
    ensure_home(app)?;
    let _one = WRITES.lock().await;
    let folio = get(app, ws, slug).await?;
    let (mut texts, entries) = contents(app, &folio.id, None).await?;
    let mut paths: BTreeSet<String> = entries.iter().map(|e| e.path.clone()).collect();
    let changed_since: BTreeSet<String> = match base_rev {
        Some(b) => revs(app, &folio.id).await?.into_iter().filter(|r| r.rev > b).flat_map(|r| r.files.into_iter().map(|f| f.path)).collect(),
        None => BTreeSet::new(),
    };
    let suffix = &uuid::Uuid::new_v4().simple().to_string()[..8];
    let mut lines = vec![];
    let mut files: Vec<(String, Option<Vec<u8>>)> = vec![];
    for op in ops {
        match op {
            Op::Write { path, data } => {
                check_path(&path)?;
                ensure!(data.len() <= MAX_FILE, "{path} is over 2 MB");
                if is_text(&path) {
                    ensure!(std::str::from_utf8(&data).is_ok(), "{path} is not UTF-8 text");
                }
                // The same content as now: nothing to keep twice.
                if is_text(&path) && texts.get(&path).is_some_and(|t| t.as_bytes() == data.as_slice()) {
                    lines.push(format!("{path} is unchanged"));
                    continue;
                }
                let path = if paths.contains(&path) && changed_since.contains(&path) {
                    let other = concurrent_path(&path, suffix);
                    lines.push(format!("{path} changed since rev {}: both versions are kept, yours as {other}. Merge them into {path} and remove {other} (launchpad folio rm).", base_rev.unwrap_or(0)));
                    other
                } else {
                    lines.push(format!("Wrote {path}"));
                    path
                };
                if is_text(&path) {
                    texts.insert(path.clone(), String::from_utf8_lossy(&data).into_owned());
                }
                paths.insert(path.clone());
                files.push((path, Some(data)));
            }
            Op::Append { path, text } => {
                check_path(&path)?;
                ensure!(is_text(&path), "append works on text files");
                let mut body = texts.get(&path).cloned().unwrap_or_default();
                if !body.is_empty() && !body.ends_with('\n') {
                    body.push('\n');
                }
                body.push_str(&text);
                if !body.ends_with('\n') {
                    body.push('\n');
                }
                ensure!(body.len() <= MAX_FILE, "{path} would be over 2 MB");
                lines.push(format!("Appended to {path}"));
                texts.insert(path.clone(), body.clone());
                paths.insert(path.clone());
                files.push((path, Some(body.into_bytes())));
            }
            Op::Patch { path, section, text } => {
                let current = texts.get(&path).cloned().with_context(|| format!("there is no page {path}"))?;
                let current = current.as_str();
                let Some(range) = check::section(current, &section) else {
                    let ids = check::section_ids(current);
                    bail!("{path} has no section `{section}` (it has {})", if ids.is_empty() { "none".into() } else { ids.join(", ") });
                };
                // The section changed since the rev the writer read: its patch goes on that rev's page, kept next
                // to the current one (both versions, like a whole write).
                if let Some(b) = base_rev.filter(|_| changed_since.contains(&path))
                    && let Ok(old) = file_text(app, &folio.id, &path, Some(b)).await
                    && let Some(old_range) = check::section(&old, &section)
                    && check::slice(&old, old_range) != check::slice(current, range)
                {
                    let body = check::replace(&old, old_range, &text);
                    ensure!(body.len() <= MAX_FILE, "{path} would be over 2 MB");
                    let other = concurrent_path(&path, suffix);
                    lines.push(format!("{path} › {section} changed since rev {b}: both versions are kept, yours (rev {b} with your patch) as {other}. Merge your change into {path} and remove {other} (launchpad folio rm)."));
                    texts.insert(other.clone(), body.clone());
                    paths.insert(other.clone());
                    files.push((other, Some(body.into_bytes())));
                    continue;
                }
                let body = check::replace(current, range, &text);
                ensure!(body.len() <= MAX_FILE, "{path} would be over 2 MB");
                lines.push(format!("Patched {path} › {section} (lines {}–{})", range.0, range.1));
                texts.insert(path.clone(), body.clone());
                files.push((path, Some(body.into_bytes())));
            }
            Op::Remove { path } => {
                ensure!(paths.contains(&path), "there is no {path} in this folio");
                ensure!(path != "index.md", "index.md is the folio's first page: rewrite it instead of removing it");
                // Removals name the content they remove: a file changed since the rev the writer read stays.
                if changed_since.contains(&path) {
                    lines.push(format!("Kept {path}: it changed since rev {}. Read it again before you remove it.", base_rev.unwrap_or(0)));
                    continue;
                }
                lines.push(format!("Removed {path} (Restore brings it back)"));
                texts.remove(&path);
                paths.remove(&path);
                files.push((path, None));
            }
        }
    }
    let marks_list = marks(app, &folio.id, None).await?;
    let problems = check::check(&check::Input { texts: texts.clone(), paths: paths.clone(), marks: mark_refs(&marks_list), slugs: Some(slugs(app, ws).await) });
    let count = |s: Severity| problems.iter().filter(|p| p.severity == s).count() as u32;
    let (errors, warnings) = (count(Severity::Error), count(Severity::Warn));
    if strict && errors > 0 {
        lines.push(format!("Refused (--strict): {}", check::counts_text(&problems)));
        return Ok(Written { folio, written: false, rev: 0, lines, problems });
    }
    let written = plain::write_files(app, &plain::folio_store(&folio.id), files, author.clone()).await?;
    if written.is_empty() {
        lines.push("Nothing changed.".into());
        return Ok(Written { folio: folio.clone(), written: false, rev: folio.rev, lines, problems });
    }
    let rev = folio.rev + 1;
    let at = now_ms();
    let title = texts.get("index.md").map(|t| page_title("index.md", t)).unwrap_or_else(|| folio.title.clone());
    let kind = texts.get("index.md").and_then(|t| check::front(t)).and_then(|m| m.get(serde_yaml_ng::Value::String("kind".into())).and_then(|v| v.as_str()).map(str::to_string)).filter(|k| cat::KINDS.iter().any(|(n, _)| n == k)).unwrap_or_else(|| folio.kind.clone());
    let session = author.session_id.clone();
    let thread = match &session {
        Some(s) if folio.thread.is_none() => app.sessions.get(s).and_then(|x| x.launch_spec.bot.as_ref().map(|b| (b.id.clone(), b.thread_id().to_string()))),
        _ => None,
    };
    let renamed: Vec<(String, String)> = texts.iter().filter(|(p, _)| check::is_page(p)).flat_map(|(_, t)| check::anchors(t).renamed).collect();
    let (fid, a, files_json, lab, sess) = (folio.id.clone(), serde_json::to_string(&author)?, serde_json::to_string(&written)?, label.map(str::to_string), session.clone());
    let created = folio.rev == 0;
    app.state_db.call(move |c| {
        let tx = c.transaction()?;
        tx.execute("INSERT INTO folio_rev (folio_id, rev, at, author, session_id, label, files, errors, warnings) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)", params![fid, rev, at, a, sess, lab, files_json, errors, warnings])?;
        tx.execute("UPDATE folio SET rev = ?2, updated_at = ?3, title = ?4, kind = ?5, problems = ?6, state = 'active', archived_at = NULL WHERE id = ?1", params![fid, rev, at, title, kind, serde_json::to_string(&[errors, warnings]).unwrap_or_default()])?;
        if let Some((bot, th)) = &thread {
            tx.execute("UPDATE folio SET thread_bot = ?2, thread_id = ?3 WHERE id = ?1 AND thread_bot IS NULL", params![fid, bot, th])?;
        }
        if let Some(s) = &sess {
            tx.execute(
                "INSERT INTO folio_link (folio_id, session_id, role, first_rev, last_rev, at) VALUES (?1, ?2, ?3, ?4, ?4, ?5)
                 ON CONFLICT (folio_id, session_id) DO UPDATE SET last_rev = ?4, at = ?5, role = CASE WHEN role = 'created' THEN role ELSE 'edited' END",
                params![fid, s, if created { "created" } else { "edited" }, rev, at],
            )?;
        }
        // renamed_from moves the owner's marks to the new id.
        for (old, new) in &renamed {
            let seq: i64 = tx.query_row("SELECT coalesce(max(seq), 0) + 1 FROM folio_mark", [], |r| r.get(0))?;
            tx.execute("UPDATE folio_mark SET target = ?3, seq = ?4 WHERE folio_id = ?1 AND target = ?2 AND deleted = 0", params![fid, old, new, seq])?;
        }
        tx.commit()?;
        Ok(())
    }).await?;
    if let (Some(s), Some(t)) = (&session, &folio.topic_id)
        && launchpad_core::scope::of_session(app, s).await.as_deref() == Some(ws)
        && let Err(e) = launchpad_core::topics::membership(app, t, std::slice::from_ref(s), true).await
    {
        tracing::warn!("folio {}: topic membership of {s}: {e:#}", folio.slug);
    }
    lines.push(format!("Written rev {rev} · {}", check::counts_text(&problems)));
    changed(app, &folio);
    let folio = get(app, ws, &folio.id).await?;
    Ok(Written { folio, written: true, rev, lines, problems })
}

/// `--fix`: applies the safe fixes of the stored folio's problems as one rev labeled "check --fix".
pub async fn fix(app: &App, ws: &str, slug: &str, author: Author) -> Result<Written> {
    let f = get(app, ws, slug).await?;
    let problems = check_stored(app, &f, None).await?;
    let (texts, _) = contents(app, &f.id, None).await?;
    let mut ops = vec![];
    let mut n = 0;
    for (path, text) in &texts {
        let mine: Vec<Problem> = problems.iter().filter(|p| &p.file == path).cloned().collect();
        let (fixed, k) = check::apply_fixes(text, &mine);
        if k > 0 {
            n += k;
            ops.push(Op::Write { path: path.clone(), data: fixed.into_bytes() });
        }
    }
    if ops.is_empty() {
        return Ok(Written { folio: f.clone(), written: false, rev: f.rev, lines: vec!["Nothing to fix.".into()], problems });
    }
    let mut w = write(app, ws, slug, ops, author, None, false, Some("check --fix")).await?;
    w.lines.insert(0, format!("Applied {n} fix{}", if n == 1 { "" } else { "es" }));
    Ok(w)
}

/// The revs of a folio, newest first.
pub async fn revs(app: &App, id: &str) -> Result<Vec<Rev>> {
    let id = id.to_string();
    app.state_db.call(move |c| {
        let mut st = c.prepare("SELECT rev, at, author, session_id, label, files, errors, warnings FROM folio_rev WHERE folio_id = ?1 ORDER BY rev DESC")?;
        let rows = st.query_map([id], |r| {
            Ok(Rev {
                rev: r.get(0)?,
                at: r.get(1)?,
                author: serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or_else(|_| Author::owner(None)),
                session_id: r.get(3)?,
                label: r.get(4)?,
                files: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
                errors: r.get(6)?,
                warnings: r.get(7)?,
            })
        })?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }).await
}

async fn blob_text(app: &App, sha: Option<&str>) -> String {
    match sha {
        Some(h) => plain::read_blob(app, h).await.ok().map(|d| String::from_utf8_lossy(&d).into_owned()).unwrap_or_default(),
        None => String::new(),
    }
}

/// The unified diff of each file of a rev.
pub async fn rev_diff(app: &App, id: &str, rev: i64) -> Result<Vec<Value>> {
    let r = revs(app, id).await?.into_iter().find(|r| r.rev == rev).context("no such rev")?;
    let mut out = vec![];
    for f in r.files {
        let patch = if is_text(&f.path) { launchpad_core::kb::unified(&blob_text(app, f.before.as_deref()).await, &blob_text(app, f.after.as_deref()).await) } else { String::new() };
        out.push(json!({"path": f.path, "patch": patch, "added": f.added, "removed": f.removed, "created": f.before.is_none(), "deleted": f.after.is_none()}));
    }
    Ok(out)
}

/// Restore: puts every file back as it was before `rev` (removing the ones it created), as a new rev by `author`.
pub async fn restore(app: &App, ws: &str, slug: &str, rev: i64, author: Author) -> Result<Written> {
    let f = get(app, ws, slug).await?;
    let r = revs(app, &f.id).await?.into_iter().find(|r| r.rev == rev).context("no such rev")?;
    let mut ops = vec![];
    for w in r.files {
        ops.push(match w.before {
            Some(h) => Op::Write { path: w.path, data: plain::read_blob(app, &h).await.context("this version is no longer kept")? },
            None => Op::Remove { path: w.path },
        });
    }
    ops.retain(|o| !matches!(o, Op::Remove { path } if path == "index.md"));
    write(app, ws, slug, ops, author, None, false, Some(&format!("restore of rev {rev}"))).await
}

/// Archive or bring back.
pub async fn set_archived(app: &App, ws: &str, slug: &str, archived: bool) -> Result<Folio> {
    let f = get(app, ws, slug).await?;
    let (id, at) = (f.id.clone(), now_ms());
    app.state_db.call(move |c| {
        // Bring back counts as a change, so it does not archive again at once.
        c.execute(
            "UPDATE folio SET state = CASE WHEN ?2 THEN 'archived' ELSE 'active' END, archived_at = CASE WHEN ?2 THEN ?3 ELSE NULL END, updated_at = CASE WHEN ?2 THEN updated_at ELSE ?3 END WHERE id = ?1",
            params![id, archived, at],
        )?;
        Ok(())
    }).await?;
    changed(app, &f);
    get(app, ws, slug).await
}

/// Deletes a folio: its store, revs, marks and links (its topic and sessions stay).
pub async fn delete(app: &App, ws: &str, slug: &str) -> Result<()> {
    let f = get(app, ws, slug).await?;
    plain::drop_store(app, &plain::folio_store(&f.id)).await?;
    let id = f.id.clone();
    app.state_db.call(move |c| {
        let tx = c.transaction()?;
        for t in ["folio_link", "folio_rev", "folio_mark"] {
            tx.execute(&format!("DELETE FROM {t} WHERE folio_id = ?1"), [&id])?;
        }
        tx.execute("DELETE FROM folio WHERE id = ?1", [&id])?;
        tx.commit()?;
        Ok(())
    }).await?;
    changed(app, &f);
    Ok(())
}

/// Recent writes of a workspace's folios (30 days, newest first): receipts in bot chats and traces.
pub async fn activity(app: &App, ws: &str) -> Result<Vec<Activity>> {
    let (ws, cutoff) = (ws.to_string(), now_ms() - 30 * 24 * 3600 * 1000);
    app.state_db.call(move |c| {
        let mut st = c.prepare("SELECT f.id, f.slug, f.title, r.rev, r.at, r.session_id, r.files FROM folio_rev r JOIN folio f ON f.id = r.folio_id WHERE f.workspace_id = ?1 AND r.at >= ?2 ORDER BY r.at DESC LIMIT 500")?;
        let rows = st.query_map(params![ws, cutoff], |r| {
            let files: Vec<plain::Written> = serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_default();
            let rev: i64 = r.get(3)?;
            Ok(Activity { folio_id: r.get(0)?, slug: r.get(1)?, title: r.get(2)?, rev, at: r.get(4)?, session_id: r.get(5)?, paths: files.iter().map(|f| f.path.clone()).collect(), created: rev == 1 })
        })?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }).await
}

fn changed(app: &App, f: &Folio) {
    app.bus.publish("folios", "changed", json!({"workspace_id": f.workspace_id, "id": f.id, "slug": f.slug}));
}

// ---------------------------------------------------------------- marks

const MARK_COLUMNS: &str = "id, seq, kind, author, page, target, quote, prefix, suffix, value, text, parent, rev, created_at, updated_at, sent_at, resolved_at, resolved_rev";

fn mark_row(r: &rusqlite::Row) -> rusqlite::Result<Mark> {
    Ok(Mark {
        id: r.get(0)?,
        seq: r.get(1)?,
        kind: r.get(2)?,
        author: serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or_else(|_| Author::owner(None)),
        page: r.get(4)?,
        target: r.get(5)?,
        quote: r.get(6)?,
        prefix: r.get(7)?,
        suffix: r.get(8)?,
        value: r.get(9)?,
        text: r.get(10)?,
        parent: r.get(11)?,
        rev: r.get(12)?,
        created_at: r.get(13)?,
        updated_at: r.get(14)?,
        sent_at: r.get(15)?,
        resolved_at: r.get(16)?,
        resolved_rev: r.get(17)?,
    })
}

/// The marks of a folio (changed after `since` with a cursor), oldest first.
pub async fn marks(app: &App, id: &str, since: Option<i64>) -> Result<Vec<Mark>> {
    let (id, since) = (id.to_string(), since.unwrap_or(0));
    app.state_db.call(move |c| {
        let mut st = c.prepare(&format!("SELECT {MARK_COLUMNS} FROM folio_mark WHERE folio_id = ?1 AND deleted = 0 AND seq > ?2 ORDER BY id"))?;
        let rows = st.query_map(params![id, since], mark_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }).await
}

/// The owner's mark: what the reader sends.
#[derive(Clone, Debug, Default, Deserialize, TS)]
#[ts(rename = "FolioMarkInput")]
pub struct MarkInput {
    /// comment, choice, verdict or check.
    pub kind: String,
    #[ts(optional)]
    pub page: Option<String>,
    #[ts(optional)]
    pub target: Option<String>,
    #[ts(optional)]
    pub quote: Option<String>,
    #[ts(optional)]
    pub prefix: Option<String>,
    #[ts(optional)]
    pub suffix: Option<String>,
    /// verdict: do, later, wont or discuss; choice: an option id; check: true or false. Empty: removes a verdict,
    /// choice or tick.
    #[ts(optional)]
    pub value: Option<String>,
    #[ts(optional)]
    pub text: Option<String>,
    /// Edits this comment (yours, not sent yet).
    #[ts(optional, type = "number")]
    pub id: Option<i64>,
}

/// Saves the owner's mark: a verdict, choice or tick replaces the previous one on the same id (unsent again); a
/// comment is new (or an edit of an unsent one). Returns the mark (None: removed).
pub async fn mark(app: &App, ws: &str, slug: &str, m: MarkInput) -> Result<Option<Mark>> {
    let f = get(app, ws, slug).await?;
    ensure!(matches!(m.kind.as_str(), "comment" | "choice" | "verdict" | "check"), "a mark is a comment, a choice, a verdict or a check");
    let value = m.value.clone().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    if m.kind == "verdict" && let Some(v) = &value {
        ensure!(cat::ADVICE.contains(&v.as_str()), "a verdict is do, later, wont or discuss");
    }
    if m.kind == "comment" {
        ensure!(m.text.as_deref().is_some_and(|t| !t.trim().is_empty()), "a comment needs text");
    } else {
        ensure!(m.target.as_deref().is_some_and(|t| !t.is_empty()), "a {} names the id it is about", m.kind);
    }
    let cap = |s: Option<String>, n: usize| s.map(|x| x.chars().take(n).collect::<String>());
    let (fid, rev, author, at) = (f.id.clone(), f.rev, serde_json::to_string(&Author::owner(None))?, now_ms());
    let id = app.state_db.call(move |c| {
        let tx = c.transaction()?;
        let seq: i64 = tx.query_row("SELECT coalesce(max(seq), 0) + 1 FROM folio_mark", [], |r| r.get(0))?;
        let existing: Option<i64> = if m.kind == "comment" {
            match m.id {
                Some(id) => {
                    let sent: Option<Option<i64>> = tx.query_row("SELECT sent_at FROM folio_mark WHERE id = ?1 AND folio_id = ?2 AND kind = 'comment' AND session_id IS NULL AND deleted = 0", params![id, fid], |r| r.get(0)).optional()?;
                    match sent {
                        Some(None) => Some(id),
                        Some(Some(_)) => return Err(anyhow::anyhow!("this comment was sent: reply to it instead")),
                        None => return Err(anyhow::anyhow!("no such comment")),
                    }
                }
                None => None,
            }
        } else {
            tx.query_row("SELECT id FROM folio_mark WHERE folio_id = ?1 AND kind = ?2 AND target = ?3 AND session_id IS NULL AND deleted = 0", params![fid, m.kind, m.target], |r| r.get(0)).optional()?
        };
        let out = match (existing, &value) {
            (Some(id), None) if m.kind != "comment" => {
                tx.execute("UPDATE folio_mark SET deleted = 1, seq = ?2, updated_at = ?3 WHERE id = ?1", params![id, seq, at])?;
                None
            }
            (None, None) if m.kind != "comment" => None,
            (Some(id), _) => {
                tx.execute("UPDATE folio_mark SET seq = ?2, value = ?3, text = coalesce(?4, text), quote = coalesce(?5, quote), page = coalesce(?6, page), updated_at = ?7, sent_at = NULL, rev = ?8 WHERE id = ?1", params![id, seq, value, cap(m.text.clone(), 4000), cap(m.quote.clone(), 400), m.page, at, rev])?;
                Some(id)
            }
            (None, _) => {
                tx.execute(
                    "INSERT INTO folio_mark (folio_id, seq, kind, author, page, target, quote, prefix, suffix, value, text, rev, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13)",
                    params![fid, seq, m.kind, author, m.page, m.target, cap(m.quote.clone(), 400), cap(m.prefix.clone(), 40), cap(m.suffix.clone(), 40), value, cap(m.text.clone(), 4000), rev, at],
                )?;
                Some(tx.last_insert_rowid())
            }
        };
        tx.commit()?;
        Ok(out)
    }).await?;
    changed(app, &f);
    Ok(match id {
        Some(i) => marks(app, &f.id, None).await?.into_iter().find(|x| x.id == i),
        None => None,
    })
}

/// Deletes the owner's comment.
pub async fn unmark(app: &App, ws: &str, slug: &str, mark: i64) -> Result<()> {
    let f = get(app, ws, slug).await?;
    let (fid, at) = (f.id.clone(), now_ms());
    let n = app.state_db.call(move |c| {
        let seq: i64 = c.query_row("SELECT coalesce(max(seq), 0) + 1 FROM folio_mark", [], |r| r.get(0))?;
        Ok(c.execute("UPDATE folio_mark SET deleted = 1, seq = ?3, updated_at = ?4 WHERE id = ?1 AND folio_id = ?2 AND session_id IS NULL", params![mark, fid, seq, at])?)
    }).await?;
    ensure!(n == 1, "no such mark of yours");
    changed(app, &f);
    Ok(())
}

/// An agent's (or the owner's) reply under a comment.
pub async fn reply(app: &App, ws: &str, slug: &str, mark: i64, text: &str, author: Author) -> Result<Mark> {
    let f = get(app, ws, slug).await?;
    ensure!(!text.trim().is_empty(), "the reply is empty");
    let list = marks(app, &f.id, None).await?;
    let parent = list.iter().find(|m| m.id == mark && m.kind == "comment").with_context(|| format!("no comment c-{mark} on this folio"))?;
    let (fid, rev, a, sess, at, t, page, target) = (f.id.clone(), f.rev, serde_json::to_string(&author)?, author.session_id.clone(), now_ms(), text.chars().take(4000).collect::<String>(), parent.page.clone(), parent.target.clone());
    let id = app.state_db.call(move |c| {
        let seq: i64 = c.query_row("SELECT coalesce(max(seq), 0) + 1 FROM folio_mark", [], |r| r.get(0))?;
        c.execute(
            "INSERT INTO folio_mark (folio_id, seq, kind, author, session_id, page, target, text, parent, rev, created_at, updated_at, sent_at) VALUES (?1, ?2, 'reply', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, ?10)",
            params![fid, seq, a, sess, page, target, t, mark, rev, at],
        )?;
        Ok(c.last_insert_rowid())
    }).await?;
    link_session(app, &f, &author, "replied").await;
    changed(app, &f);
    marks(app, &f.id, None).await?.into_iter().find(|m| m.id == id).context("the reply disappeared")
}

/// Resolves a comment (naming the rev that addressed it, default the latest), or opens it again.
pub async fn resolve(app: &App, ws: &str, slug: &str, mark: i64, rev: Option<i64>, open: bool, author: Author) -> Result<Mark> {
    let f = get(app, ws, slug).await?;
    let rev = rev.unwrap_or(f.rev);
    ensure!(rev >= 1 && rev <= f.rev, "rev {rev} does not exist (the folio is at rev {})", f.rev);
    let (fid, at) = (f.id.clone(), now_ms());
    let n = app.state_db.call(move |c| {
        let seq: i64 = c.query_row("SELECT coalesce(max(seq), 0) + 1 FROM folio_mark", [], |r| r.get(0))?;
        Ok(c.execute(
            "UPDATE folio_mark SET resolved_at = CASE WHEN ?3 THEN NULL ELSE ?4 END, resolved_rev = CASE WHEN ?3 THEN NULL ELSE ?5 END, seq = ?6, updated_at = ?4 WHERE id = ?1 AND folio_id = ?2 AND kind = 'comment' AND deleted = 0",
            params![mark, fid, open, at, rev, seq],
        )?)
    }).await?;
    ensure!(n == 1, "no comment c-{mark} on this folio");
    link_session(app, &f, &author, "replied").await;
    changed(app, &f);
    marks(app, &f.id, None).await?.into_iter().find(|m| m.id == mark).context("the comment disappeared")
}

async fn link_session(app: &App, f: &Folio, author: &Author, role: &str) {
    let Some(s) = author.session_id.clone() else { return };
    let (fid, rev, at, role) = (f.id.clone(), f.rev, now_ms(), role.to_string());
    let _ = app.state_db.call(move |c| Ok(c.execute("INSERT INTO folio_link (folio_id, session_id, role, first_rev, last_rev, at) VALUES (?1, ?2, ?3, ?4, ?4, ?5) ON CONFLICT (folio_id, session_id) DO UPDATE SET at = ?5", params![fid, s, role, rev, at])?)).await;
    if let (Some(t), Some(s)) = (&f.topic_id, &author.session_id) {
        let _ = launchpad_core::topics::membership(app, t, std::slice::from_ref(s), true).await;
    }
}

/// The owner's marks for an agent: the sent ones changed after `since` (all sent ones without), with the replies,
/// and the cursor to pass next time.
pub async fn feedback(app: &App, ws: &str, slug: &str, since: Option<i64>) -> Result<Value> {
    let f = get(app, ws, slug).await?;
    let all = marks(app, &f.id, None).await?;
    let cursor = all.iter().map(|m| m.seq).max().unwrap_or(0);
    let mine: Vec<Mark> = all.iter().filter(|m| m.author.kind == AuthorKind::Owner && m.kind != "reply" && m.sent_at.is_some() && since.is_none_or(|s| m.seq > s)).cloned().collect();
    let replies: Vec<Mark> = all.iter().filter(|m| m.kind == "reply").cloned().collect();
    let unsent = all.iter().filter(|m| m.author.kind == AuthorKind::Owner && m.kind != "reply" && m.sent_at.is_none()).count();
    Ok(json!({"text": feedback_text(&f.slug, f.rev, &mine, &replies), "marks": mine, "replies": replies, "cursor": cursor, "rev": f.rev, "unsent": unsent}))
}

/// Typed items of a folio merged with the owner's marks (`launchpad folio items`): each with its block, page, line,
/// fields and the owner's verdict, choice, tick and open comments.
pub async fn items(app: &App, ws: &str, slug: &str, kind: Option<&str>, verdict: Option<&str>, open_only: bool) -> Result<Vec<Value>> {
    let f = get(app, ws, slug).await?;
    let (texts, _) = contents(app, &f.id, None).await?;
    let ms = marks(app, &f.id, None).await?;
    let mut out = vec![];
    for path in check::page_order(&texts) {
        let text = &texts[&path];
        let ls = split::lines(text);
        for b in check::blocks(text) {
            let (Some(def), Some(v)) = (b.def, &b.value) else { continue };
            if kind.is_some_and(|k| k != def.name) {
                continue;
            }
            for (id, item) in check::typed_ids(def, v) {
                let mark = |k: &str| ms.iter().find(|m| m.kind == k && m.target.as_deref() == Some(id.as_str()) && m.author.kind == AuthorKind::Owner).and_then(|m| m.value.clone());
                let comments: Vec<Value> = ms.iter().filter(|m| m.kind == "comment" && m.target.as_deref() == Some(id.as_str()) && m.resolved_at.is_none()).map(|m| json!({"id": m.id, "text": m.text})).collect();
                let v = mark("verdict");
                if verdict.is_some_and(|want| v.as_deref() != Some(want)) || (open_only && comments.is_empty() && v.is_none() && mark("choice").is_none() && mark("check").is_none()) {
                    continue;
                }
                let line = ls.iter().enumerate().skip(b.line as usize).take((b.end - b.line) as usize).find(|(_, l)| l.contains(&format!("id: {id}"))).map_or(b.line, |(i, _)| i as u32 + 1);
                out.push(json!({"id": id, "block": def.name, "page": path, "line": line, "fields": serde_json::to_value(&item).unwrap_or(Value::Null), "verdict": v, "choice": mark("choice"), "check": mark("check"), "comments": comments}));
            }
        }
    }
    Ok(out)
}

/// Where Send feedback can go: the folio's bot thread, the sessions that wrote it (newest first).
#[derive(Clone, Debug, Serialize, TS)]
#[ts(rename = "FolioTarget")]
pub struct Target {
    /// thread or session.
    pub kind: String,
    pub bot_id: Option<String>,
    pub thread_id: Option<String>,
    pub session_id: Option<String>,
    pub label: String,
    pub detail: String,
}

pub async fn targets(app: &App, ws: &str, slug: &str) -> Result<Vec<Target>> {
    let f = get(app, ws, slug).await?;
    let mut out = vec![];
    if let Some(t) = &f.thread
        && let Ok(bot) = launchpad_core::bots::get(app, &t.bot_id).await
    {
        let title = launchpad_core::bot_threads::get(app, &t.bot_id, &t.thread_id).await.ok().flatten().map(|x| x.title).unwrap_or_else(|| f.title.clone());
        out.push(Target { kind: "thread".into(), bot_id: Some(t.bot_id.clone()), thread_id: Some(t.thread_id.clone()), session_id: None, label: bot.settings.name.clone(), detail: format!("thread “{title}”") });
    }
    for l in &f.links {
        let Some(s) = app.sessions.get(&l.session_id) else { continue };
        if s.launch_spec.bot.is_some() {
            continue;
        }
        out.push(Target { kind: "session".into(), bot_id: None, thread_id: None, session_id: Some(s.id.clone()), label: format!("Session “{}”", s.title), detail: format!("{} · rev {}", l.role, l.last_rev) });
    }
    Ok(out)
}

/// Where the owner sends the feedback.
#[derive(Clone, Debug, Deserialize, TS)]
#[ts(rename = "FolioSendTo")]
pub struct SendTo {
    /// thread (the folio's, or `bot_id` + `thread_id`), session, or new_thread (a new thread of `bot_id`, with the
    /// folio's topic as context; it becomes the folio's thread).
    pub kind: String,
    #[ts(optional)]
    pub bot_id: Option<String>,
    #[ts(optional)]
    pub thread_id: Option<String>,
    #[ts(optional)]
    pub session_id: Option<String>,
    /// A line of your own, before the summary.
    #[ts(optional)]
    pub note: Option<String>,
}

/// One Send feedback at a time: two clicks never send the same marks twice.
static SENDS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Send feedback: one short message to the folio's thread, a session that wrote it, or a new thread of a bot; the
/// unsent marks become sent once the message went. Returns the session that got it.
pub async fn send(app: &App, ws: &str, slug: &str, to: SendTo) -> Result<Value> {
    let _one = SENDS.lock().await;
    let f = get(app, ws, slug).await?;
    let all = marks(app, &f.id, None).await?;
    let unsent: Vec<Mark> = all.iter().filter(|m| m.author.kind == AuthorKind::Owner && m.kind != "reply" && m.sent_at.is_none()).cloned().collect();
    let note = to.note.as_deref().map(str::trim).filter(|n| !n.is_empty());
    ensure!(!unsent.is_empty() || note.is_some(), "nothing to send: comment, decide or mark items first");
    let since = unsent.iter().map(|m| m.seq).min().map_or(0, |s| s - 1);
    let mut text = feedback_message(&f.title, &f.slug, f.rev, &unsent, since);
    if let Some(n) = note {
        text = format!("{n}\n\n{text}");
    }
    let session = deliver(app, ws, &f, &to, text, Default::default()).await?;
    // Only the marks the message counted: a mark saved or changed meanwhile stays unsent.
    let ids: Vec<(i64, i64)> = unsent.iter().map(|m| (m.id, m.seq)).collect();
    let at = now_ms();
    app.state_db.call(move |c| {
        let tx = c.transaction()?;
        for (id, seq) in ids {
            tx.execute("UPDATE folio_mark SET sent_at = ?2 WHERE id = ?1 AND seq = ?3", params![id, at, seq])?;
        }
        tx.commit()?;
        Ok(())
    }).await?;
    changed(app, &f);
    Ok(json!({"session_id": session.id, "bot_id": session.launch_spec.bot.as_ref().map(|b| b.id.clone()), "thread": session.launch_spec.bot.as_ref().map(|b| b.thread_id().to_string()), "sent": unsent.len()}))
}

/// The line "Ask the author to fix" sends (the problems panel): it sends no marks.
pub fn fix_message(title: &str, slug: &str, rev: i64, errors: u32, warnings: u32) -> String {
    let n = |c: u32, one: &str| format!("{c} {one}{}", if c == 1 { "" } else { "s" });
    format!("Fix the problems of the folio “{title}” (rev {rev}: {}, {}): run `launchpad folio check {slug}`, fix every error (`launchpad folio check {slug} --fix` applies the safe fixes, `launchpad folio patch` the rest), then check again. The folio: /folios/{slug}", n(errors, "error"), n(warnings, "warning"))
}

/// Ask the author to fix: one line to the same recipients as Send feedback; the marks stay as they are.
pub async fn ask_fix(app: &App, ws: &str, slug: &str, to: SendTo) -> Result<Value> {
    let _one = SENDS.lock().await;
    let f = get(app, ws, slug).await?;
    let mut text = fix_message(&f.title, &f.slug, f.rev, f.errors, f.warnings);
    if let Some(n) = to.note.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        text = format!("{n}\n\n{text}");
    }
    let session = deliver(app, ws, &f, &to, text, Default::default()).await?;
    Ok(sent(&session))
}

fn sent(session: &launchpad_core::model::Session) -> Value {
    json!({"session_id": session.id, "bot_id": session.launch_spec.bot.as_ref().map(|b| b.id.clone()), "thread": session.launch_spec.bot.as_ref().map(|b| b.thread_id().to_string())})
}

/// The reader's Ask: the owner's question about a folio (or a page, a section, a quote of it).
#[derive(Clone, Debug, Deserialize, TS)]
#[ts(rename = "FolioAsk")]
pub struct Ask {
    pub text: String,
    #[ts(optional)]
    pub page: Option<String>,
    #[ts(optional)]
    pub section: Option<String>,
    #[ts(optional)]
    pub quote: Option<String>,
    /// The rev the owner read.
    #[ts(optional, type = "number")]
    pub rev: Option<i64>,
    /// Where it goes, like Send feedback (default: the folio's thread, else its key `folio:<id>` of `bot_id`).
    #[serde(flatten)]
    #[ts(flatten)]
    pub to: SendTo,
}

/// Ask: the owner's question goes to the folio's thread (or a session that wrote it, or a new thread of a bot), with
/// the folio (its page, section and the selected quote) as context, a reference Home resolves. Marks stay unsent.
pub async fn ask(app: &App, ws: &str, slug: &str, a: Ask) -> Result<Value> {
    let f = get(app, ws, slug).await?;
    let text = a.text.trim().to_string();
    ensure!(!text.is_empty(), "type your question");
    let mut context = launchpad_core::model::MessageContext::default();
    context.folios.push(launchpad_core::model::FolioRef { id: f.id.clone(), slug: f.slug.clone(), title: f.title.clone(), rev: a.rev, page: a.page, section: a.section.filter(|s| !s.is_empty()), items: vec![], quote: a.quote });
    let session = deliver(app, ws, &f, &a.to, text, context).await?;
    Ok(sent(&session))
}

/// The owner's message about a folio to its thread, a session of the workspace or a new thread of a bot (which
/// becomes the folio's thread, with the folio's topic as context). The session that got it joins the topic.
async fn deliver(app: &App, ws: &str, f: &Folio, to: &SendTo, text: String, mut context: launchpad_core::model::MessageContext) -> Result<launchpad_core::model::Session> {
    use launchpad_core::bot_threads::{self, Source, To};
    let session = match to.kind.as_str() {
        "thread" | "new_thread" => {
            let bot = to.bot_id.clone().or_else(|| f.thread.as_ref().map(|t| t.bot_id.clone())).context("choose a bot")?;
            let b = launchpad_core::bots::get(app, &bot).await?;
            ensure!(b.workspace_id == f.workspace_id, "this bot belongs to another workspace");
            let new = to.kind == "new_thread";
            // A thread that no longer exists (its bot's threads were cleaned up): the folio's key finds or starts one.
            let named = to.thread_id.clone().or_else(|| f.thread.as_ref().filter(|t| t.bot_id == bot).map(|t| t.thread_id.clone()));
            let named = match named {
                Some(t) if bot_threads::get(app, &bot, &t).await.ok().flatten().is_some() => Some(t),
                _ => None,
            };
            let dest = match named {
                _ if new => bot_threads::route(Source::Folio { id: &f.id, title: &f.title, new: true }),
                Some(t) => To::Thread(t),
                None => bot_threads::route(Source::Folio { id: &f.id, title: &f.title, new: false }),
            };
            if new && let Some(t) = &f.topic_id && let Ok(topic) = launchpad_core::topics::get(app, t).await {
                context.topics.push(launchpad_core::topics::TopicContext { id: topic.id, name: topic.name, snapshot: None, notes: vec![] });
            }
            let s = launchpad_core::bots::message_with(app, &bot, launchpad_core::bots::Message { text, attachments: vec![], context, automatic: false, key: None, to: dest }).await?;
            // The thread that got it is the folio's thread from now on (a new one, or one that continued).
            if let Some(b) = &s.launch_spec.bot {
                let (fid, bot, th) = (f.id.clone(), b.id.clone(), b.thread_id().to_string());
                app.state_db.call(move |c| Ok(c.execute("UPDATE folio SET thread_bot = ?2, thread_id = ?3 WHERE id = ?1", params![fid, bot, th])?)).await?;
            }
            s
        }
        "session" => {
            let id = to.session_id.clone().context("choose a session")?;
            ensure!(launchpad_core::scope::of_session(app, &id).await.as_deref() == Some(ws), "this session belongs to another workspace");
            launchpad_core::runners::send_anywhere_with(app, &id, &text, vec![], context, None).await?;
            app.sessions.get(&id).context("the session disappeared")?
        }
        k => bail!("unknown target {k:?}: thread, session or new_thread"),
    };
    if let Some(t) = &f.topic_id {
        let _ = launchpad_core::topics::membership(app, t, std::slice::from_ref(&session.id), true).await;
    }
    Ok(session)
}

// ---------------------------------------------------------------- auto-archive

/// Pure: whether a folio archives itself now: active, no change and no mark activity for 30 days, no open mark.
pub fn stale(state: &str, updated_at: i64, last_mark: Option<i64>, open: u32, unsent: u32, now: i64) -> bool {
    state == "active" && open == 0 && unsent == 0 && now - updated_at.max(last_mark.unwrap_or(0)) >= ARCHIVE_AFTER_MS
}

/// Archives the folios with no change and no open mark for 30 days (hourly, on Home).
pub async fn archive_stale(app: &App) -> Result<usize> {
    let rows: Vec<(String, String, String, i64, Option<i64>)> = app.state_db.call(|c| {
        let mut st = c.prepare("SELECT f.id, f.workspace_id, f.state, f.updated_at, (SELECT max(updated_at) FROM folio_mark m WHERE m.folio_id = f.id) FROM folio f WHERE f.state = 'active'")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }).await?;
    let mut n = 0;
    for (id, ws, state, updated, last_mark) in rows {
        let Ok(f) = get(app, &ws, &id).await else { continue };
        if stale(&state, updated, last_mark, f.open, f.unsent, now_ms()) {
            set_archived(app, &ws, &id, true).await?;
            n += 1;
        }
    }
    Ok(n)
}

pub fn spawn(app: &App) {
    if !launchpad_core::home_git::here(app) {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            tick.tick().await;
            match archive_stale(&app).await {
                Ok(0) => {}
                Ok(n) => tracing::info!("archived {n} folio(s) without a change for 30 days"),
                Err(e) => tracing::warn!("folio auto-archive: {e:#}"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner_mark(kind: &str, target: &str, value: &str) -> Mark {
        Mark { id: 31, seq: 5, kind: kind.into(), author: Author::owner(None), page: Some("plan.md".into()), target: Some(target.into()), quote: Some("Comments wait for wave 2".into()), prefix: None, suffix: None, value: Some(value.into()), text: Some("Could we ship page-level notes first?".into()), parent: None, rev: 3, created_at: 0, updated_at: 0, sent_at: Some(1), resolved_at: None, resolved_rev: None }
    }

    #[test]
    fn slugs_and_paths() {
        assert_eq!(slugify("Launchpad roadmap, winter 2026!"), "launchpad-roadmap-winter-2026");
        assert_eq!(slugify("¿¿"), "folio");
        assert!(valid_slug("roadmap-winter") && !valid_slug("Roadmap") && !valid_slug("-x"));
        assert!(check_path("assets/flow.png").is_ok());
        for bad in ["../x", "/etc/passwd", ".git/config", "a//b", "a\\b", ".hidden"] {
            assert!(check_path(bad).is_err(), "{bad}");
        }
        assert_eq!(concurrent_path("plan.md", "abcd1234"), "plan.concurrent-abcd1234.md");
    }

    #[test]
    fn feedback_reads_in_a_few_lines() {
        let marks = vec![owner_mark("comment", "waves", ""), owner_mark("verdict", "F-01", "do"), owner_mark("verdict", "F-03", "later"), owner_mark("choice", "D-SHARE", "after-30-days")];
        let t = feedback_text("roadmap-winter", 14, &marks, &[]);
        assert!(t.starts_with("folio roadmap-winter · rev 14 · 4 marks from the owner"), "{t}");
        assert!(t.contains("comment c-31  plan.md › #waves  \"Comments wait for wave 2\"\n  Could we ship page-level notes first?"), "{t}");
        assert!(t.contains("verdict  F-01 do · F-03 later"), "{t}");
        assert!(t.contains("choice   D-SHARE → after-30-days"), "{t}");
        assert!(t.contains("next: launchpad folio read roadmap-winter plan.md --section waves"), "{t}");
        let m = feedback_message("Roadmap", "roadmap-winter", 14, &marks, 812);
        assert!(m.starts_with("Feedback on the folio “Roadmap” (rev 14): 1 comment, 2 verdicts, 1 decision. Read it with `launchpad folio feedback roadmap-winter --since 812`"), "{m}");
    }

    #[test]
    fn archives_only_quiet_folios() {
        let day = 24 * 3600 * 1000;
        let now = 100 * day;
        assert!(stale("active", now - 31 * day, None, 0, 0, now));
        assert!(!stale("active", now - 29 * day, None, 0, 0, now));
        assert!(!stale("active", now - 31 * day, Some(now - day), 0, 0, now));
        assert!(!stale("active", now - 31 * day, None, 1, 0, now));
        assert!(!stale("active", now - 31 * day, None, 0, 2, now));
        assert!(!stale("archived", now - 31 * day, None, 0, 0, now));
    }

    #[test]
    fn page_titles() {
        assert_eq!(page_title("index.md", "---\ntitle: Roadmap\n---\n# Other\n"), "Roadmap");
        assert_eq!(page_title("plan.md", "Intro\n\n## Plan: three waves\n"), "Plan: three waves");
        assert_eq!(page_title("notes/research.md", "text"), "research");
    }
}

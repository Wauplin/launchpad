//! Knowledge base: one per workspace, the only memory agents have.
//!
//! Home stores it like a plain folder (store `kb-<workspace id>`: a tree in `state.db`, contents as blobs that
//! Home backups upload, every overwrite and delete kept 30 days). Home serializes writes and records who made
//! each change (session, model, runner, or you) and how many lines it added and removed.
//!
//! - Runners keep a mirror in `<data_dir>/knowledge/<workspace name>/`, synced every minute and before a
//!   session starts. Jails mount it read-only; agents read it as files and write with `launchpad kb`. Your own
//!   edits in the mirror (for example in Obsidian) go up as yours.
//! - A write states the content it started from. When the note changed since, both versions are kept: the
//!   new one as `<name>.concurrent-<id>.<ext>` until you or the gardener merge them.
//! - Harness memory is off in every session; the adapter's instruction file points to the knowledge base and
//!   carries `INDEX.md`.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use ts_rs::TS;

use crate::App;
use crate::plain::{self, Meta, Update};

pub const INDEX: &str = "INDEX.md";
/// Lines of `INDEX.md` given to an agent at start (about 2k tokens).
const INDEX_LINES: usize = 150;
/// Notes larger than this are not searched or diffed.
const TEXT_MAX: usize = 1 << 20;

const SEED_INDEX: &str = "# Index\n\nThe map of this knowledge base: one line per important note, linking to it. Agents read this file at the start of every session, so keep it under 150 lines.\n";
const SEED_AGENTS: &str = r#"# How to write here

This is the knowledge base of this workspace, the only memory agents have. Agents read it at the start of sessions, so every line costs context: keep it small.

## What to write

- A few durable, high-level findings: the owner's preferences and decisions with their reasons, non-obvious facts that took effort to discover, how-tos, people and project context.
- Anything the owner asks you to remember.

## What not to write

- No log: no record of sessions or runs, task status, what you did, what went well or failed. Traces, Git history and the session list already keep that.
- Nothing a repository already says (README, CLAUDE.md, AGENTS.md), nothing you can find again quickly, nothing true only for the current task.

Most sessions write nothing. Before you add a note, update or remove an existing one.

## Layout

- `INDEX.md`: the map, loaded at the start of every session. Keep it under 150 lines; add a line when you add an important note.
- `people/`, `projects/`, `topics/`, `decisions/`, `howto/`: one Markdown note per subject, with a short kebab-case file name.
- `sources/`: captures of pages or documents, with their URL and date.
- `media/`: images, video and audio, added with `launchpad kb add-media`.
- `bots/<bot>/`: a bot's own notes, short and current (preferences and facts it needs next time, never a journal of its runs).

## Notes

Start each note with front matter:

```
---
title: Xet upload retries
type: decision        # fact | decision | howto | person | project | source
status: current       # current | superseded
valid_from: 2026-10-12
sources:
  - trace: 4ec4108f#212
  - url: https://github.com/huggingface/huggingface_hub/issues/3412
---
```

Link notes with [[wikilinks]]. When a fact changes, update the note, or mark the old one `status: superseded` and link the new one. Prefer updating an existing note over creating a near duplicate.

## Writing

Files here are read-only: write with `launchpad kb write|append|add-media|rm`. Every change records who made it and can be restored for 30 days. A file named `<note>.concurrent-<id>.md` is a second version written while the note changed: merge it into the note, then remove it.
"#;

const GARDENER_PROMPT: &str = "Tend the knowledge base of this workspace (its folder is $LAUNCHPAD_KB; read AGENTS.md there first). Merge duplicate notes, cut notes down to their durable findings (remove logs of sessions or runs, task status and results that only mattered once), add missing [[wikilinks]], mark outdated notes `status: superseded` with a link to the current one, merge every `*.concurrent-*` file into its note and remove it, and keep INDEX.md an accurate map under 150 lines. Change only what clearly improves the notes, write with `launchpad kb`, and end with a short list of what you changed.";

pub fn store(workspace_id: &str) -> String {
    format!("kb-{workspace_id}")
}

pub fn workspace_of(store: &str) -> Option<&str> {
    store.strip_prefix("kb-")
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AuthorKind {
    Agent,
    Owner,
    Launchpad,
}

/// Who made a knowledge base change.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Author {
    pub kind: AuthorKind,
    /// The session title, "You", "Launchpad" or where an import came from.
    pub name: String,
    pub session_id: Option<String>,
    pub model: Option<String>,
    /// The runner's name.
    pub runner: Option<String>,
}

impl Author {
    pub fn owner(detail: Option<String>) -> Author {
        Author { kind: AuthorKind::Owner, name: detail.unwrap_or_else(|| "You".into()), session_id: None, model: None, runner: None }
    }
    fn launchpad() -> Author {
        Author { kind: AuthorKind::Launchpad, name: "Launchpad".into(), session_id: None, model: None, runner: None }
    }
}

/// One write. Contents (`sha256`) are uploaded as blobs first. `base`: the content the writer started from
/// (None: it expected no file).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    Write { path: String, sha256: String, size: u64, base: Option<String> },
    Append { path: String, text: String },
    Remove { path: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Concurrent {
    /// The second version.
    pub path: String,
    /// The note it belongs to.
    pub note: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Notes {
    pub workspace_id: String,
    #[ts(type = "number")]
    pub version: i64,
    pub files: Vec<plain::Entry>,
    pub concurrent: Vec<Concurrent>,
    /// The change you last marked seen.
    #[ts(type = "number")]
    pub seen: i64,
    /// The prompt of the gardener, for the composer.
    pub gardener_prompt: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Hit {
    pub path: String,
    pub line: u32,
    pub text: String,
}

/// A memory file of a harness on a runner, offered for import.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct NativeMemory {
    /// Where it goes in the knowledge base: `imported/<harness>/<group>/<file>`.
    pub path: String,
    /// Where it is on the runner.
    pub source: String,
    pub content: String,
}

/// `name.concurrent-<id>.ext` → `name.ext`.
pub fn concurrent_note(path: &str) -> Option<String> {
    let (dir, file) = path.rsplit_once('/').map(|(d, f)| (Some(d), f)).unwrap_or((None, path));
    let i = file.find(".concurrent-")?;
    let rest = &file[i + ".concurrent-".len()..];
    let (id, ext) = rest.split_once('.').map(|(a, b)| (a, Some(b))).unwrap_or((rest, None));
    if id.len() != 8 || !id.bytes().all(|b| b.is_ascii_hexdigit()) || i == 0 {
        return None;
    }
    let name = format!("{}{}", &file[..i], ext.map(|e| format!(".{e}")).unwrap_or_default());
    Some(match dir {
        Some(d) => format!("{d}/{name}"),
        None => name,
    })
}

fn concurrent_path(path: &str, id: &str) -> String {
    let (dir, file) = path.rsplit_once('/').map(|(d, f)| (format!("{d}/"), f)).unwrap_or((String::new(), path));
    match file.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{dir}{stem}.concurrent-{id}.{ext}"),
        _ => format!("{dir}{file}.concurrent-{id}"),
    }
}

fn sha(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

fn text(data: &[u8]) -> Option<&str> {
    (data.len() <= TEXT_MAX).then(|| std::str::from_utf8(data).ok()).flatten()
}

// ---------------------------------------------------------------- line diff

#[derive(Debug, PartialEq)]
enum D<'a> {
    Same(&'a str),
    Del(&'a str),
    Add(&'a str),
}

/// A line diff (longest common subsequence); large files fall back to "all removed, all added".
fn diff_lines<'a>(old: &'a str, new: &'a str) -> Vec<D<'a>> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let (mut pre, mut post) = (0, 0);
    while pre < a.len() && pre < b.len() && a[pre] == b[pre] {
        pre += 1;
    }
    while post < a.len() - pre && post < b.len() - pre && a[a.len() - 1 - post] == b[b.len() - 1 - post] {
        post += 1;
    }
    let (am, bm) = (&a[pre..a.len() - post], &b[pre..b.len() - post]);
    let mut out: Vec<D> = a[..pre].iter().map(|l| D::Same(l)).collect();
    if am.len() * bm.len() > 4_000_000 {
        out.extend(am.iter().map(|l| D::Del(l)));
        out.extend(bm.iter().map(|l| D::Add(l)));
    } else {
        let (n, m) = (am.len(), bm.len());
        let mut t = vec![0u32; (n + 1) * (m + 1)];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                t[i * (m + 1) + j] = if am[i] == bm[j] { t[(i + 1) * (m + 1) + j + 1] + 1 } else { t[(i + 1) * (m + 1) + j].max(t[i * (m + 1) + j + 1]) };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n || j < m {
            if i < n && j < m && am[i] == bm[j] {
                out.push(D::Same(am[i]));
                i += 1;
                j += 1;
            } else if i < n && (j == m || t[(i + 1) * (m + 1) + j] >= t[i * (m + 1) + j + 1]) {
                out.push(D::Del(am[i]));
                i += 1;
            } else {
                out.push(D::Add(bm[j]));
                j += 1;
            }
        }
    }
    out.extend(a[a.len() - post..].iter().map(|l| D::Same(l)));
    out
}

/// Lines added and removed.
pub(crate) fn counts(old: &str, new: &str) -> (u32, u32) {
    diff_lines(old, new).iter().fold((0, 0), |(a, r), d| match d {
        D::Add(_) => (a + 1, r),
        D::Del(_) => (a, r + 1),
        D::Same(_) => (a, r),
    })
}

/// A unified diff with three lines of context.
pub fn unified(old: &str, new: &str) -> String {
    let d = diff_lines(old, new);
    let changed: Vec<usize> = d.iter().enumerate().filter(|(_, x)| !matches!(x, D::Same(_))).map(|(i, _)| i).collect();
    let mut out = String::new();
    let mut k = 0;
    while k < changed.len() {
        let start = changed[k].saturating_sub(3);
        let mut end = changed[k] + 3;
        while k + 1 < changed.len() && changed[k + 1] <= end + 4 {
            k += 1;
            end = changed[k] + 3;
        }
        k += 1;
        let end = end.min(d.len() - 1);
        // Line numbers of the hunk start, counted over the lines before it.
        let (mut o, mut n) = (1, 1);
        for x in &d[..start] {
            match x {
                D::Same(_) => (o, n) = (o + 1, n + 1),
                D::Del(_) => o += 1,
                D::Add(_) => n += 1,
            }
        }
        let olen = d[start..=end].iter().filter(|x| !matches!(x, D::Add(_))).count();
        let nlen = d[start..=end].iter().filter(|x| !matches!(x, D::Del(_))).count();
        out.push_str(&format!("@@ -{},{olen} +{},{nlen} @@\n", if olen == 0 { o - 1 } else { o }, if nlen == 0 { n - 1 } else { n }));
        for x in &d[start..=end] {
            match x {
                D::Same(l) => out.push_str(&format!(" {l}\n")),
                D::Del(l) => out.push_str(&format!("-{l}\n")),
                D::Add(l) => out.push_str(&format!("+{l}\n")),
            }
        }
    }
    out
}

// ---------------------------------------------------------------- Home

static WRITES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn ensure_home(app: &App) -> Result<()> {
    ensure!(crate::home_git::here(app), "the knowledge base is stored by Home");
    Ok(())
}

async fn blob_text(app: &App, sha: &str) -> Option<String> {
    let data = plain::read_blob(app, sha).await.ok()?;
    text(&data).map(str::to_string)
}

async fn put(app: &App, data: &[u8]) -> Result<(String, u64)> {
    let h = sha(data);
    plain::write_blob(app, &h, data).await?;
    Ok((h, data.len() as u64))
}

/// Home: a new knowledge base starts with `INDEX.md` and `AGENTS.md`.
async fn seed(app: &App, workspace_id: &str) -> Result<()> {
    let id = store(workspace_id);
    let t = plain::tree(app, &id).await?;
    if t.version > 0 || !t.files.is_empty() {
        return Ok(());
    }
    let mut updates = vec![];
    for (path, body) in [(INDEX, SEED_INDEX), ("AGENTS.md", SEED_AGENTS)] {
        let (h, size) = put(app, body.as_bytes()).await?;
        updates.push((Update { path: path.into(), sha256: Some(h), size, executable: false }, Some(Meta { author: Author::launchpad(), added: Some(body.lines().count() as u32), removed: Some(0) })));
    }
    plain::record(app, &id, None, updates).await?;
    Ok(())
}

/// Home: applies writes in order, one writer at a time. Returns a line per write.
pub async fn apply(app: &App, workspace_id: &str, author: Author, ops: Vec<Op>) -> Result<Vec<String>> {
    ensure_home(app)?;
    crate::workspace::get(app, workspace_id).await?;
    let _one = WRITES.lock().await;
    seed(app, workspace_id).await?;
    let id = store(workspace_id);
    let mut files: HashMap<String, plain::Entry> = plain::tree(app, &id).await?.files.into_iter().map(|e| (e.path.clone(), e)).collect();
    let mut updates = vec![];
    let mut out = vec![];
    let suffix = &sha(format!("{:?}{}", author.session_id, crate::util::now_ms()).as_bytes())[..8];
    for op in ops {
        let (path, new, size) = match op {
            Op::Write { path, sha256, size, base } => {
                plain::check_path(&path)?;
                ensure!(plain::blob_path(app, &sha256)?.exists(), "the content of {path} was not uploaded");
                let current = files.get(&path).map(|e| e.sha256.clone());
                if current.as_deref() == Some(sha256.as_str()) {
                    out.push(format!("{path}: unchanged"));
                    continue;
                }
                if current.is_some() && current != base {
                    let other = concurrent_path(&path, suffix);
                    out.push(match author.kind {
                        AuthorKind::Agent => format!("{path} changed since you read it: both versions are kept, yours as {other}. Merge them into {path} and remove {other} (launchpad kb rm), or leave it for the owner."),
                        _ => format!("{path} changed meanwhile: both versions are kept, yours as {other}. Open {path} to keep one."),
                    });
                    (other, Some(sha256), size)
                } else {
                    out.push(format!("Wrote {path}"));
                    (path, Some(sha256), size)
                }
            }
            Op::Append { path, text: add } => {
                plain::check_path(&path)?;
                let mut body = match files.get(&path) {
                    Some(e) => blob_text(app, &e.sha256).await.with_context(|| format!("{path} is not a text note"))?,
                    None => String::new(),
                };
                if !body.is_empty() && !body.ends_with('\n') {
                    body.push('\n');
                }
                body.push_str(&add);
                if !body.ends_with('\n') {
                    body.push('\n');
                }
                let (h, size) = put(app, body.as_bytes()).await?;
                out.push(format!("Appended to {path}"));
                (path, Some(h), size)
            }
            Op::Remove { path } => {
                ensure!(files.contains_key(&path), "there is no {path} in the knowledge base");
                out.push(format!("Removed {path} (restorable for 30 days)"));
                (path, None, 0)
            }
        };
        let content = |h: Option<String>| async move {
            match h {
                Some(h) => blob_text(app, &h).await,
                None => Some(String::new()),
            }
        };
        let old_text = content(files.get(&path).map(|e| e.sha256.clone())).await;
        let new_text = content(new.clone()).await;
        let (added, removed) = match (&old_text, &new_text) {
            (Some(o), Some(n)) => {
                let (a, r) = counts(o, n);
                (Some(a), Some(r))
            }
            _ => (None, None),
        };
        match &new {
            Some(h) => files.insert(path.clone(), plain::Entry { path: path.clone(), sha256: h.clone(), size, executable: false }),
            None => files.remove(&path),
        };
        updates.push((Update { path, sha256: new, size, executable: false }, Some(Meta { author: author.clone(), added, removed })));
    }
    if !updates.is_empty() {
        plain::record(app, &id, author.session_id.as_deref(), updates).await?;
    }
    Ok(out)
}

/// Home: moves every note under `from/` to `to/` (a renamed bot's notes) and points INDEX.md's links at the new
/// folder, as one change by Launchpad. Nothing to move: nothing changes.
pub async fn move_folder(app: &App, workspace_id: &str, from: &str, to: &str) -> Result<()> {
    let (from, to) = (format!("{}/", from.trim_end_matches('/')), format!("{}/", to.trim_end_matches('/')));
    let tree = plain::tree(app, &store(workspace_id)).await?;
    let mut ops = vec![];
    for e in tree.files.iter().filter(|e| e.path.starts_with(&from)) {
        ops.push(Op::Write { path: format!("{to}{}", &e.path[from.len()..]), sha256: e.sha256.clone(), size: e.size, base: None });
        ops.push(Op::Remove { path: e.path.clone() });
    }
    if ops.is_empty() {
        return Ok(());
    }
    if let Some(index) = tree.files.iter().find(|e| e.path == "INDEX.md")
        && let Some(text) = blob_text(app, &index.sha256).await
        && text.contains(&from)
    {
        let (h, size) = put(app, text.replace(&from, &to).as_bytes()).await?;
        ops.push(Op::Write { path: "INDEX.md".into(), sha256: h, size, base: Some(index.sha256.clone()) });
    }
    apply(app, workspace_id, Author::launchpad(), ops).await?;
    Ok(())
}

/// Home: the author of a session's writes, from Home's own records.
async fn session_author(app: &App, session_id: &str) -> Result<(Author, String)> {
    let s = app.sessions.get(session_id).context("no such session")?;
    let runner = match &s.runner_id {
        Some(r) => crate::runners::list(app).await.ok().and_then(|l| l.into_iter().find(|x| &x.runner.id == r)).map(|x| x.runner.name),
        None => None,
    };
    let ws = workspace_of_session(app, session_id).await.context("this session has no workspace, so no knowledge base")?;
    Ok((Author { kind: AuthorKind::Agent, name: s.title.clone(), session_id: Some(s.id.clone()), model: s.model.clone(), runner }, ws))
}

/// Home: writes a runner sends for one of its sessions (`launchpad kb` through the tunnel).
pub async fn runner_apply(app: &App, runner_id: &str, body: Value) -> Result<Value> {
    let ops: Vec<Op> = serde_json::from_value(body["ops"].clone())?;
    let (author, ws) = if body["owner"].as_bool() == Some(true) {
        // Your edits in a runner's mirror.
        let runner = crate::runners::list(app).await.ok().and_then(|l| l.into_iter().find(|x| x.runner.id == runner_id)).map(|x| x.runner.name);
        (Author { runner, ..Author::owner(None) }, body["workspace_id"].as_str().context("workspace_id")?.to_string())
    } else {
        let session_id = body["session_id"].as_str().context("session_id")?;
        crate::home_git::check_session(app, runner_id, session_id)?;
        session_author(app, session_id).await?
    };
    Ok(json!({"lines": apply(app, &ws, author, ops).await?}))
}

/// Home: a knowledge base gets its first files before anyone reads it.
pub async fn ensure_seeded(app: &App, workspace_id: &str) -> Result<()> {
    ensure_home(app)?;
    crate::workspace::get(app, workspace_id).await?;
    let _one = WRITES.lock().await;
    seed(app, workspace_id).await
}

pub async fn notes(app: &App, workspace_id: &str) -> Result<Notes> {
    ensure_seeded(app, workspace_id).await?;
    let t = plain::tree(app, &store(workspace_id)).await?;
    let concurrent = t.files.iter().filter_map(|e| Some(Concurrent { note: concurrent_note(&e.path)?, path: e.path.clone() })).collect();
    let ws = workspace_id.to_string();
    let seen = app.state_db.call(move |c| Ok(c.query_row("SELECT change_id FROM kb_seen WHERE workspace_id = ?1", [ws], |r| r.get(0)).optional()?.unwrap_or(0))).await?;
    Ok(Notes { workspace_id: workspace_id.into(), version: t.version, files: t.files, concurrent, seen, gardener_prompt: GARDENER_PROMPT.into() })
}

/// The content of a note (or a previous version, by hash) that this knowledge base holds or held.
pub async fn read(app: &App, workspace_id: &str, path: Option<&str>, sha256: Option<&str>) -> Result<Vec<u8>> {
    ensure_home(app)?;
    let id = store(workspace_id);
    let sha = match (path, sha256) {
        (_, Some(h)) => {
            let known = plain::tree(app, &id).await?.files.iter().any(|e| e.sha256 == h) || plain::history(app, &id).await?.iter().any(|c| c.sha256.as_deref() == Some(h) || c.new_sha256.as_deref() == Some(h));
            ensure!(known, "this version is no longer kept");
            h.to_string()
        }
        (Some(p), None) => plain::tree(app, &id).await?.files.into_iter().find(|e| e.path == p).with_context(|| format!("there is no {p}"))?.sha256,
        (None, None) => bail!("name a note"),
    };
    plain::read_blob(app, &sha).await
}

pub async fn changes(app: &App, workspace_id: &str) -> Result<Vec<plain::Change>> {
    ensure_home(app)?;
    plain::history(app, &store(workspace_id)).await
}

/// The unified diff of one change.
pub async fn diff(app: &App, workspace_id: &str, change: i64) -> Result<String> {
    let c = changes(app, workspace_id).await?.into_iter().find(|c| c.id == change).context("this change is no longer kept")?;
    let get = |h: Option<String>| async move {
        match h {
            Some(h) => blob_text(app, &h).await,
            None => Some(String::new()),
        }
    };
    let (old, new) = (get(c.sha256).await, get(c.new_sha256).await);
    match (old, new) {
        (Some(o), Some(n)) => Ok(unified(&o, &n)),
        _ => Ok(String::new()),
    }
}

/// Restore previous: puts back the content from before a change (a note that change created is removed). The
/// restore is itself a change, by you.
pub async fn restore(app: &App, workspace_id: &str, change: i64) -> Result<Vec<String>> {
    let c = changes(app, workspace_id).await?.into_iter().find(|c| c.id == change).context("this change is no longer kept")?;
    let current = plain::tree(app, &store(workspace_id)).await?.files.into_iter().find(|e| e.path == c.path).map(|e| e.sha256);
    let op = match c.sha256 {
        Some(h) => {
            let size = tokio::fs::metadata(plain::blob_path(app, &h)?).await.map(|m| m.len()).unwrap_or(0);
            Op::Write { path: c.path, sha256: h, size, base: current }
        }
        None => Op::Remove { path: c.path },
    };
    apply(app, workspace_id, Author::owner(None), vec![op]).await
}

/// Your edit from the Knowledge page. `base`: the content you started from.
pub async fn owner_write(app: &App, workspace_id: &str, path: &str, content: &[u8], base: Option<String>) -> Result<Vec<String>> {
    ensure_home(app)?;
    let (h, size) = put(app, content).await?;
    apply(app, workspace_id, Author::owner(None), vec![Op::Write { path: path.into(), sha256: h, size, base }]).await
}

/// A concurrent edit: keep the note as it is (`use_other` false) or replace it with the second version; the
/// second version is removed either way.
pub async fn resolve(app: &App, workspace_id: &str, path: &str, use_other: bool) -> Result<Vec<String>> {
    let note = concurrent_note(path).context("this is not a concurrent edit")?;
    let files = plain::tree(app, &store(workspace_id)).await?.files;
    let other = files.iter().find(|e| e.path == path).context("this concurrent edit is already resolved")?;
    let mut ops = vec![];
    if use_other {
        let base = files.iter().find(|e| e.path == note).map(|e| e.sha256.clone());
        ops.push(Op::Write { path: note, sha256: other.sha256.clone(), size: other.size, base });
    }
    ops.push(Op::Remove { path: path.into() });
    apply(app, workspace_id, Author::owner(None), ops).await
}

/// Imports memory files (from a runner's harnesses), as yours.
pub async fn import(app: &App, workspace_id: &str, from: &str, files: Vec<(String, String)>) -> Result<Vec<String>> {
    ensure_home(app)?;
    let mut ops = vec![];
    for (path, content) in files {
        plain::check_path(&path)?;
        let (h, size) = put(app, content.as_bytes()).await?;
        ops.push(Op::Write { path, sha256: h, size, base: None });
    }
    ensure!(!ops.is_empty(), "choose at least one file");
    let n = ops.len();
    apply(app, workspace_id, Author::owner(Some(format!("Imported from {from}"))), ops).await?;
    Ok(vec![format!("Imported {n} files")])
}

pub async fn mark_seen(app: &App, workspace_id: &str, change: i64) -> Result<()> {
    let ws = workspace_id.to_string();
    app.state_db.call(move |c| Ok(c.execute("INSERT INTO kb_seen (workspace_id, change_id) VALUES (?1, ?2) ON CONFLICT (workspace_id) DO UPDATE SET change_id = max(change_id, ?2)", params![ws, change])?)).await?;
    app.bus.publish("kb", "seen", json!({"workspace_id": workspace_id}));
    Ok(())
}

/// Changes by agents and Launchpad you have not looked at, per workspace.
pub async fn unseen(app: &App) -> Result<BTreeMap<String, u32>> {
    if !crate::home_git::here(app) {
        return Ok(BTreeMap::new());
    }
    let cutoff = crate::util::now_ms() - 30 * 24 * 3600 * 1000;
    let rows: Vec<(String, Option<String>)> = app.state_db.call(move |c| {
        let mut st = c.prepare("SELECT h.store, h.meta FROM plain_history h LEFT JOIN kb_seen s ON s.workspace_id = substr(h.store, 4) WHERE h.store LIKE 'kb-%' AND h.at >= ?1 AND h.id > coalesce(s.change_id, 0)")?;
        let rows = st.query_map([cutoff], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }).await?;
    let mut out = BTreeMap::new();
    for (s, meta) in rows {
        let by_owner = meta.and_then(|m| serde_json::from_str::<Meta>(&m).ok()).is_some_and(|m| m.author.kind != AuthorKind::Agent);
        if !by_owner && let Some(ws) = workspace_of(&s) {
            *out.entry(ws.to_string()).or_insert(0) += 1;
        }
    }
    Ok(out)
}

/// Lines containing every word of `query` (case-insensitive), in text notes.
pub fn search_text<'a>(files: impl Iterator<Item = (String, &'a str)>, query: &str, limit: usize) -> Vec<Hit> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut out = vec![];
    if words.is_empty() {
        return out;
    }
    for (path, body) in files {
        let in_path = path.to_lowercase();
        for (i, line) in body.lines().enumerate() {
            let l = line.to_lowercase();
            if words.iter().all(|w| l.contains(w.as_str()) || in_path.contains(w.as_str())) && words.iter().any(|w| l.contains(w.as_str())) {
                out.push(Hit { path: path.clone(), line: i as u32 + 1, text: line.chars().take(300).collect() });
                if out.len() >= limit {
                    return out;
                }
            }
        }
    }
    out
}

pub async fn search(app: &App, workspace_id: &str, query: &str) -> Result<Vec<Hit>> {
    ensure_home(app)?;
    let mut bodies = vec![];
    for e in plain::tree(app, &store(workspace_id)).await?.files.into_iter().filter(|e| (e.size as usize) <= TEXT_MAX) {
        if let Some(t) = blob_text(app, &e.sha256).await {
            bodies.push((e.path, t));
        }
    }
    Ok(search_text(bodies.iter().map(|(p, t)| (p.clone(), t.as_str())), query, 200))
}

// ---------------------------------------------------------------- runner side

#[derive(Default, Serialize, Deserialize)]
struct Synced {
    dir: PathBuf,
    version: i64,
    files: BTreeMap<String, String>,
}

fn state_file(app: &App, workspace_id: &str) -> PathBuf {
    app.paths.data_dir.join("knowledge-state").join(format!("{workspace_id}.json"))
}

/// The folder name of a workspace's mirror: its name, readable in Obsidian.
pub(crate) fn folder_name(name: &str, id: &str) -> String {
    let clean: String = name.chars().map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.') { c } else { '-' }).collect();
    let clean = clean.trim().trim_matches('.').to_string();
    if clean.is_empty() { id.to_string() } else { clean }
}

/// Where this machine keeps a workspace's mirror.
pub async fn mirror_dir(app: &App, workspace_id: &str) -> Result<PathBuf> {
    let w = crate::workspace::get(app, workspace_id).await?;
    let root = app.paths.data_dir.join("knowledge");
    let mut dir = root.join(folder_name(&w.name, &w.id));
    // Two workspaces whose names clean up to the same folder.
    for other in crate::workspace::list(app).await? {
        if other.id != w.id && folder_name(&other.name, &other.id) == folder_name(&w.name, &w.id) && other.created_at < w.created_at {
            dir = root.join(format!("{} {}", folder_name(&w.name, &w.id), &w.id[..8.min(w.id.len())]));
        }
    }
    Ok(dir)
}

/// Runner: sends the owner's edits in the mirror, then brings it to Home's version. Returns the mirror.
pub async fn sync(app: &App, workspace_id: &str) -> Result<PathBuf> {
    static SYNCS: std::sync::LazyLock<std::sync::Mutex<HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>> = std::sync::LazyLock::new(Default::default);
    let gate = SYNCS.lock().unwrap().entry(workspace_id.to_string()).or_default().clone();
    let _one = gate.lock().await;
    let id = store(workspace_id);
    let dir = mirror_dir(app, workspace_id).await?;
    let state = state_file(app, workspace_id);
    let mut synced: Synced = tokio::fs::read(&state).await.ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    // A renamed workspace: the mirror follows its name.
    if !synced.dir.as_os_str().is_empty() && synced.dir != dir && synced.dir.is_dir() && !dir.exists() {
        tokio::fs::create_dir_all(dir.parent().unwrap()).await?;
        tokio::fs::rename(&synced.dir, &dir).await?;
    }
    tokio::fs::create_dir_all(&dir).await?;
    if synced.dir != dir {
        synced = Synced { dir: dir.clone(), ..Default::default() };
    }
    // Your edits since the last sync.
    let mut ops = vec![];
    let mut local = BTreeMap::new();
    for rel in plain::walk(&dir) {
        if plain::check_path(&rel).is_err() || rel.split('/').any(|c| c.starts_with('.')) {
            continue;
        }
        let (h, size, _) = plain::hash_file(&dir.join(&rel)).await?;
        if synced.files.get(&rel) != Some(&h) && !synced.files.is_empty() {
            plain::home::upload(app, &id, &h, tokio::fs::read(dir.join(&rel)).await?).await?;
            ops.push(Op::Write { path: rel.clone(), sha256: h.clone(), size, base: synced.files.get(&rel).cloned() });
        }
        local.insert(rel, h);
    }
    for rel in synced.files.keys().filter(|r| !local.contains_key(*r)) {
        ops.push(Op::Remove { path: rel.clone() });
    }
    if !ops.is_empty() {
        crate::home_link::home_call("kb", json!({"workspace_id": workspace_id, "owner": true, "ops": ops})).await?;
    }
    // Home's version.
    let tree = plain::home::tree(app, &id).await?;
    let wanted: BTreeMap<&str, &plain::Entry> = tree.files.iter().map(|e| (e.path.as_str(), e)).collect();
    for (rel, h) in &local {
        // Files Home no longer has, unless they are yours and new (sent above, so Home has them).
        if !wanted.contains_key(rel.as_str()) && synced.files.get(rel) == Some(h) {
            let _ = tokio::fs::remove_file(dir.join(rel)).await;
        }
    }
    for e in &tree.files {
        plain::check_path(&e.path)?;
        if local.get(&e.path) == Some(&e.sha256) {
            continue;
        }
        let path = dir.join(&e.path);
        let data = plain::home::blob(app, &id, &e.sha256).await?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let tmp = path.with_extension("lp-partial");
        tokio::fs::write(&tmp, data).await?;
        tokio::fs::rename(&tmp, &path).await?;
    }
    remove_empty_dirs(&dir).await;
    let synced = Synced { dir: dir.clone(), version: tree.version, files: tree.files.iter().map(|e| (e.path.clone(), e.sha256.clone())).collect() };
    tokio::fs::create_dir_all(state.parent().unwrap()).await?;
    tokio::fs::write(&state, serde_json::to_vec(&synced)?).await?;
    Ok(dir)
}

async fn remove_empty_dirs(root: &Path) {
    let mut dirs: Vec<PathBuf> = ignore::WalkBuilder::new(root).hidden(false).git_ignore(false).build().flatten().filter(|e| e.file_type().is_some_and(|t| t.is_dir())).map(|e| e.into_path()).filter(|p| p != root).collect();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    for d in dirs {
        let _ = tokio::fs::remove_dir(&d).await;
    }
}

/// Runners keep every workspace's mirror current.
pub fn spawn(app: &App) {
    if !app.paths.role.runs_agents() || !cfg!(unix) {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
        let mut failing: HashMap<String, String> = HashMap::new();
        loop {
            tick.tick().await;
            if crate::home_link::load(&app.paths).is_none() {
                continue;
            }
            for w in crate::workspace::list(&app).await.unwrap_or_default() {
                match sync(&app, &w.id).await {
                    Ok(_) => {
                        failing.remove(&w.id);
                    }
                    Err(e) => {
                        let e = format!("{e:#}");
                        if failing.get(&w.id) != Some(&e) {
                            tracing::warn!("knowledge base of {}: {e}", w.name);
                            failing.insert(w.id.clone(), e);
                        }
                    }
                }
            }
        }
    });
}

/// The workspace of a session on this machine.
pub async fn workspace_of_session(app: &App, session_id: &str) -> Option<String> {
    let s = app.sessions.get(session_id)?;
    match &s.project_id {
        Some(p) => crate::projects::get(app, p).await.ok().flatten().and_then(|p| p.workspace_id).or(s.launch_spec.workspace_id.clone()),
        None => s.launch_spec.workspace_id.clone(),
    }
}

/// What the agent is told about its knowledge base, with `INDEX.md`.
pub async fn instructions(app: &App, workspace_id: &str, dir: &Path) -> String {
    let name = crate::workspace::get(app, workspace_id).await.map(|w| w.name).unwrap_or_default();
    let index = tokio::fs::read_to_string(dir.join(INDEX)).await.unwrap_or_default();
    let lines: Vec<&str> = index.lines().collect();
    let shown = lines.iter().take(INDEX_LINES).copied().collect::<Vec<_>>().join("\n");
    let more = if lines.len() > INDEX_LINES { format!("\n\n(INDEX.md continues: {} more lines in the file.)", lines.len() - INDEX_LINES) } else { String::new() };
    let cli = app.paths.cli_command();
    format!(
        "# Knowledge base\n\nYour memory is the Launchpad knowledge base of the workspace \"{name}\", in `{dir}` (also `$LAUNCHPAD_KB`). Your harness's own memory is off: do not write memory files anywhere else.\n\n- Read it as files: start from INDEX.md below, look up with `rg` in that folder or `{cli} kb search <words>`.\n- The files are read-only. Write with `{cli} kb write <path>` (content on stdin, or `--file <file>`), `{cli} kb append <path>`, `{cli} kb add-media <file>` and `{cli} kb rm <path>`. Read AGENTS.md there before your first write.\n- Write rarely: only a few durable, high-level findings (the owner's preferences and decisions with their reasons, non-obvious facts that took effort to discover, how-tos), and what the owner asks you to remember. Never keep a log of sessions or runs, task status, what you did, or what went well or failed: traces and Git history keep that. Most sessions write nothing. Every session reads this, so keep notes short, update or remove an existing note instead of adding one, and keep INDEX.md current. Repository instructions (CLAUDE.md, AGENTS.md in a repository) are not memory and stay where they are.\n\n## INDEX.md\n\n{shown}{more}\n",
        dir = dir.display()
    )
}

/// `launchpad kb …` from a session on this machine: uploads contents and sends the writes to Home, then
/// refreshes the mirror.
pub async fn session_write(app: &App, session_id: &str, ops: Vec<LocalOp>) -> Result<Vec<String>> {
    let ws = workspace_of_session(app, session_id).await.context("this session has no workspace, so no knowledge base")?;
    let id = store(&ws);
    let dir = mirror_dir(app, &ws).await?;
    let state: Synced = tokio::fs::read(state_file(app, &ws)).await.ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let mut sent = vec![];
    for op in ops {
        sent.push(match op {
            LocalOp::Write { path, content_b64 } => {
                let path = clean_path(&path)?;
                let data = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, content_b64)?;
                let h = sha(&data);
                let size = data.len() as u64;
                plain::home::upload(app, &id, &h, data).await?;
                Op::Write { base: state.files.get(&path).cloned(), path, sha256: h, size }
            }
            LocalOp::Append { path, text } => Op::Append { path: clean_path(&path)?, text },
            LocalOp::Remove { path } => Op::Remove { path: clean_path(&path)? },
        });
    }
    let v = crate::home_link::home_call("kb", json!({"session_id": session_id, "ops": sent})).await?;
    let lines: Vec<String> = serde_json::from_value(v["lines"].clone()).unwrap_or_default();
    if let Err(e) = sync(app, &ws).await {
        tracing::warn!("knowledge base mirror {}: {e:#}", dir.display());
    }
    Ok(lines)
}

/// A path given to `launchpad kb`: relative to the knowledge base, or an absolute path inside the mirror.
fn clean_path(path: &str) -> Result<String> {
    let p = path.trim().trim_start_matches("./").to_string();
    plain::check_path(&p).map(|_| p)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum LocalOp {
    Write { path: String, content_b64: String },
    Append { path: String, text: String },
    Remove { path: String },
}

/// Runner: the memory files of this machine's harnesses (their own logins and homes), for a one-time import.
pub async fn native_memories(app: &App) -> Vec<NativeMemory> {
    let Some(home) = dirs::home_dir() else { return vec![] };
    let mut out = vec![];
    for r in crate::harness::registrations().iter() {
        let id = r.descriptor.id;
        for (group, dir) in app.harnesses.get(id).native_memories(&home) {
            for rel in plain::walk(&dir) {
                if !rel.ends_with(".md") {
                    continue;
                }
                let Ok(content) = tokio::fs::read_to_string(dir.join(&rel)).await else { continue };
                if content.trim().is_empty() || content.len() > TEXT_MAX {
                    continue;
                }
                let path = format!("imported/{}/{}/{rel}", r.descriptor.name.to_lowercase().replace(' ', "-"), folder_name(&group, "memory"));
                if plain::check_path(&path).is_ok() {
                    out.push(NativeMemory { path, source: dir.join(&rel).to_string_lossy().into_owned(), content });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_names() {
        assert_eq!(concurrent_path("topics/xet.md", "1a2b3c4d"), "topics/xet.concurrent-1a2b3c4d.md");
        assert_eq!(concurrent_path("README", "1a2b3c4d"), "README.concurrent-1a2b3c4d");
        assert_eq!(concurrent_note("topics/xet.concurrent-1a2b3c4d.md").as_deref(), Some("topics/xet.md"));
        assert_eq!(concurrent_note("README.concurrent-1a2b3c4d").as_deref(), Some("README"));
        assert_eq!(concurrent_note("topics/xet.md"), None);
        assert_eq!(concurrent_note("topics/.concurrent-1a2b3c4d.md"), None);
        assert_eq!(concurrent_note("a.concurrent-zz.md"), None);
    }

    #[test]
    fn line_diffs() {
        assert_eq!(counts("a\nb\nc\n", "a\nB\nc\nd\n"), (2, 1));
        assert_eq!(counts("", "x\ny\n"), (2, 0));
        assert_eq!(counts("x\n", ""), (0, 1));
        let old = (1..=20).map(|i| format!("l{i}\n")).collect::<String>();
        let new = old.replace("l2\n", "L2\n").replace("l18\n", "");
        let patch = unified(&old, &new);
        assert_eq!(patch.matches("@@ ").count(), 2, "{patch}");
        assert!(patch.starts_with("@@ -1,5 +1,5 @@\n l1\n-l2\n+L2\n"), "{patch}");
        assert!(patch.contains("@@ -15,6 +15,5 @@\n"), "{patch}");
        assert_eq!(unified("same\n", "same\n"), "");
        assert_eq!(unified("", "new\n"), "@@ -0,0 +1,1 @@\n+new\n");
    }

    #[test]
    fn search_needs_every_word() {
        let files = vec![("topics/xet.md".to_string(), "Uploads retry 5 times\nnothing here"), ("people/ana.md".to_string(), "Ana owns uploads")];
        let hits = search_text(files.into_iter(), "uploads retry", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].path.as_str(), hits[0].line), ("topics/xet.md", 1));
        // A word may match the file name.
        assert_eq!(search_text(vec![("topics/xet.md".to_string(), "Uploads retry")].into_iter(), "xet retry", 10).len(), 1);
    }
}

//! Code viewer (ARCHITECTURE.md, section 8.1): sources, file lists with change letters, file contents
//! with changed line ranges, and `git grep` search. Read only.
//!
//! Path checks (section 14): a root is a registered project or one of its git worktrees, a path is
//! relative without `..`, and a file read from disk must resolve (after symlinks) inside its root.
//! A revision is resolved to a commit id first (after `--end-of-options`), and later git commands only
//! get that id, so git never reads user input as an option.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncBufReadExt;
use ts_rs::TS;

use crate::App;
use crate::model::Project;
use crate::shellenv::UserEnv;

/// Larger files are not loaded.
pub const MAX_LOAD: u64 = 5 * 1024 * 1024;
const MAX_HITS: usize = 1000;

/// One source of the viewer: a working tree, or a revision read through the project checkout.
pub struct Source {
    pub project: Project,
    pub root: PathBuf,
    /// The revision as given (`origin/main`) and its commit id.
    pub rev: Option<(String, String)>,
    pub git: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct CodeSources {
    pub git: bool,
    /// `origin/main`: the base for change letters.
    pub default_branch: Option<String>,
    /// The project checkout first.
    pub worktrees: Vec<Worktree>,
    /// Local and remote branches, tags and fetched PR heads, most recent first.
    pub refs: Vec<RefInfo>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Worktree {
    pub path: String,
    pub branch: Option<String>,
    pub main: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct RefInfo {
    /// What to pass as `rev`: `main`, `origin/main`, `v1.3`, `refs/launchpad/pr/3420`.
    pub name: String,
    /// `branch`, `remote`, `tag` or `pr`.
    pub kind: String,
    pub label: String,
    #[ts(type = "number")]
    pub date: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct FileList {
    pub git: bool,
    pub files: Vec<String>,
    /// Path → letter: M modified, A added, U untracked, since `base`.
    pub changes: BTreeMap<String, String>,
    /// The branch the changes are counted from (`origin/main`, or `HEAD` without a default branch).
    pub base: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct CodeFile {
    pub path: String,
    #[ts(type = "number")]
    pub size: u64,
    /// `text`, `image`, `binary` or `too_large`.
    pub kind: String,
    /// Shiki language id and display name.
    pub language: Option<String>,
    pub language_name: Option<String>,
    pub text: Option<String>,
    pub marks: Vec<LineMark>,
    pub added: u32,
    pub removed: u32,
    /// M, A or U since `base`.
    pub letter: Option<String>,
    pub base: Option<String>,
    /// The working tree file differs from `HEAD`, so a GitHub link would point at other lines.
    pub dirty: bool,
}

/// Changed lines of the open file (1-based, inclusive). `kind`: `add`, `mod` or `del` (lines were
/// removed after `start`).
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct LineMark {
    pub start: u32,
    pub end: u32,
    pub kind: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct SearchQuery {
    pub q: String,
    #[serde(default)]
    pub case: bool,
    #[serde(default)]
    pub word: bool,
    #[serde(default)]
    pub regex: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct SearchHit {
    pub path: String,
    pub line: u32,
    /// The line, cut around the match when it is very long.
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct SearchResult {
    pub hits: Vec<SearchHit>,
    /// More than 1,000 results: the rest are not listed.
    pub truncated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct CommitInfo {
    pub sha: Option<String>,
    /// `owner/repo` of a remote branch that contains the commit, for GitHub permalinks.
    pub github: Option<String>,
}

// ---------------------------------------------------------------- sources and checks

/// Checks a root (and resolves a revision) before anything is read.
pub async fn open(app: &App, root: &str, rev: Option<&str>) -> Result<Source> {
    let root = std::fs::canonicalize(root).with_context(|| format!("{root} does not exist"))?;
    let projects = crate::projects::list(app).await?;
    let project = match projects.iter().find(|p| Path::new(&p.path) == root) {
        Some(p) => p.clone(),
        None => main_worktree(&root)
            .and_then(|main| projects.into_iter().find(|p| Path::new(&p.path) == main))
            .with_context(|| format!("{} is not a registered project or one of its worktrees", root.display()))?,
    };
    let git = crate::git::git_dir(&root).is_some();
    let rev = match rev.map(str::trim).filter(|r| !r.is_empty()) {
        None => None,
        Some(r) => {
            if !git {
                bail!("{} is not a git repository", root.display());
            }
            let spec = format!("{r}^{{commit}}");
            let sha = run(&app.env, &root, &["rev-parse", "--verify", "--quiet", "--end-of-options", &spec])
                .await
                .ok()
                .filter(|s| !s.is_empty())
                .with_context(|| format!("unknown revision {r}"))?;
            Some((r.to_string(), sha))
        }
    };
    Ok(Source { project, root, rev, git })
}

/// Resolve hexadecimal object IDs only, without accepting refs named like hashes or peeling tags.
/// Unknown, ambiguous and non-commit objects are omitted.
pub async fn resolve_commits(env: &UserEnv, src: &Source, oids: &[String]) -> Result<BTreeMap<String, String>> {
    if oids.len() > 64 || oids.iter().any(|oid| !(7..=40).contains(&oid.len()) || !oid.bytes().all(|b| b.is_ascii_hexdigit())) {
        bail!("expected at most 64 hexadecimal commit IDs of 7–40 characters");
    }
    if !src.git {
        return Ok(BTreeMap::new());
    }
    resolve_commit_ids(env, &src.root, oids).await
}

async fn resolve_commit_ids(env: &UserEnv, root: &Path, oids: &[String]) -> Result<BTreeMap<String, String>> {
    let mut found = BTreeMap::new();
    for oid in oids {
        if found.contains_key(oid) {
            continue;
        }
        let prefix = format!("--disambiguate={}", oid.to_ascii_lowercase());
        let matches = run(env, root, &["rev-parse", &prefix]).await?;
        let mut matches = matches.lines();
        if let Some(sha) = matches.next()
            && matches.next().is_none()
            && run(env, root, &["cat-file", "-t", sha]).await? == "commit"
        {
            found.insert(oid.clone(), sha.to_string());
        }
    }
    Ok(found)
}

/// Commit changes are read against the first parent (or the empty tree for a root commit).
/// The summary carries no patches; viewers load one selected file at a time.
pub async fn commit_diff(app: &App, src: &Source) -> Result<serde_json::Value> {
    let sha = &src.rev.as_ref().context("a commit revision is required")?.1;
    commit_diff_at(&app.env, &src.root, sha).await
}

async fn commit_diff_at(env: &UserEnv, root: &Path, sha: &str) -> Result<serde_json::Value> {
    let bytes = run_raw(env, root, &["show", "-s", "--format=%s%n%P", sha, "--"]).await?;
    let info = String::from_utf8_lossy(&bytes);
    let mut info = info.lines();
    let subject = info.next().unwrap_or_default();
    let parent = info.next().and_then(|parents| parents.split_whitespace().next());
    let mut files = commit_changed_files(env, root, sha).await?;
    let stats = run_raw(
        env,
        root,
        &[
            "show",
            "--format=",
            "--root",
            "--first-parent",
            "--diff-merges=first-parent",
            "--find-renames",
            "--no-ext-diff",
            "--no-textconv",
            "--numstat",
            "-z",
            sha,
            "--",
        ],
    )
    .await?;
    let mut parts = stats.split(|b| *b == 0).filter(|p| !p.is_empty());
    while let Some(part) = parts.next() {
        let text = String::from_utf8_lossy(part);
        let mut fields = text.splitn(3, '\t');
        let additions = fields.next().and_then(|s| s.parse::<u64>().ok());
        let deletions = fields.next().and_then(|s| s.parse::<u64>().ok());
        let path = fields.next().unwrap_or_default();
        let path = if path.is_empty() {
            parts.next(); // Old path of a rename; the file list is keyed by the new path.
            parts.next().map(|p| String::from_utf8_lossy(p).into_owned()).unwrap_or_default()
        } else {
            path.to_string()
        };
        if let Some(file) = files.iter_mut().find(|f| f["filename"] == path)
            && let (Some(additions), Some(deletions)) = (additions, deletions)
        {
            file["additions"] = serde_json::json!(additions);
            file["deletions"] = serde_json::json!(deletions);
        }
    }
    Ok(serde_json::json!({"sha": sha, "subject": subject, "parent": parent, "files": files}))
}

async fn commit_changed_files(env: &UserEnv, root: &Path, sha: &str) -> Result<Vec<serde_json::Value>> {
    let bytes = run_raw(
        env,
        root,
        &[
            "show",
            "--format=",
            "--root",
            "--first-parent",
            "--diff-merges=first-parent",
            "--find-renames",
            "--no-ext-diff",
            "--no-textconv",
            "--name-status",
            "-z",
            sha,
            "--",
        ],
    )
    .await?;
    let paths = split_z(&bytes);
    let mut paths = paths.iter();
    let mut files = Vec::new();
    while let Some(status) = paths.next() {
        let old = paths.next().context("missing changed file path")?;
        let (path, previous, status) = match status.as_bytes().first() {
            Some(b'R' | b'C') => (paths.next().context("missing rename path")?, Some(old), "renamed"),
            Some(b'A') => (old, None, "added"),
            Some(b'D') => (old, None, "removed"),
            _ => (old, None, "modified"),
        };
        let mut file = serde_json::json!({"filename": path, "status": status});
        if let Some(previous) = previous {
            file["previous_filename"] = serde_json::json!(previous);
        }
        files.push(file);
    }
    Ok(files)
}

pub async fn commit_diff_file(app: &App, src: &Source, path: &str) -> Result<serde_json::Value> {
    check_path(path)?;
    let sha = &src.rev.as_ref().context("a commit revision is required")?.1;
    commit_diff_file_at(&app.env, &src.root, sha, path).await
}

async fn commit_diff_file_at(env: &UserEnv, root: &Path, sha: &str, path: &str) -> Result<serde_json::Value> {
    let files = commit_changed_files(env, root, sha).await?;
    let mut file = files.into_iter().find(|f| f["filename"] == path).context("no such file in this commit's diff")?;
    let mut args = vec![
        "show",
        "--format=",
        "--root",
        "--first-parent",
        "--diff-merges=first-parent",
        "--find-renames",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "--src-prefix=a/",
        "--dst-prefix=b/",
        sha,
        "--",
        path,
    ];
    if let Some(previous) = file["previous_filename"].as_str() {
        args.push(previous);
    }
    let patch = run_raw(env, root, &args).await?;
    let parsed = crate::items::parse_diff(&String::from_utf8_lossy(&patch));
    file["patch"] = parsed.iter().find(|f| f["filename"] == path).map(|f| f["patch"].clone()).unwrap_or(serde_json::json!(""));
    file["language"] = serde_json::json!(language(path).map(|(id, _)| id));
    Ok(file)
}

/// The main checkout of a linked worktree, checked against git's own records: the worktree's `.git`
/// file points to `<main>/.git/worktrees/<name>`, whose `gitdir` file points back to the worktree.
pub(crate) fn main_worktree(root: &Path) -> Option<PathBuf> {
    let dir = crate::git::git_dir(root)?;
    let back = std::fs::read_to_string(dir.join("gitdir")).ok()?;
    if std::fs::canonicalize(back.trim()).ok()? != std::fs::canonicalize(root.join(".git")).ok()? {
        return None;
    }
    let common = std::fs::read_to_string(dir.join("commondir")).ok()?;
    let common = std::fs::canonicalize(dir.join(common.trim())).ok()?;
    Some(common.parent()?.to_path_buf())
}

/// A relative path without `..`, as given by the UI.
fn check_path(path: &str) -> Result<&Path> {
    let p = Path::new(path);
    if path.is_empty() || path.contains('\0') || !p.components().all(|c| matches!(c, Component::Normal(_))) {
        bail!("invalid path {path}");
    }
    Ok(p)
}

/// The file on disk, after following symlinks, must stay inside the root.
fn inside(root: &Path, rel: &Path) -> Result<PathBuf> {
    let full = std::fs::canonicalize(root.join(rel)).with_context(|| format!("{} does not exist", rel.display()))?;
    if !full.starts_with(root) {
        bail!("{} is outside the folder", rel.display());
    }
    Ok(full)
}

async fn run(env: &UserEnv, root: &Path, args: &[&str]) -> Result<String> {
    crate::procs::output(env, "git", args, Some(root)).await
}

/// Untrimmed stdout, for `-z` lists and file contents.
async fn run_raw(env: &UserEnv, root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let out = crate::procs::command(env, "git", Some(root))
        .args(["--literal-pathspecs", "-c", "core.quotepath=off"])
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .context("run git")?;
    if !out.status.success() {
        bail!("git {} failed: {}", args.first().unwrap_or(&""), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(out.stdout)
}

fn split_z(bytes: &[u8]) -> Vec<String> {
    bytes.split(|b| *b == 0).filter(|s| !s.is_empty()).map(|s| String::from_utf8_lossy(s).into_owned()).collect()
}

/// `origin/HEAD`, then the usual names.
async fn default_branch(env: &UserEnv, root: &Path) -> Option<String> {
    if let Ok(b) = run(env, root, &["symbolic-ref", "--quiet", "--short", "refs/remotes/origin/HEAD"]).await
        && !b.is_empty()
    {
        return Some(b);
    }
    let names = ["refs/remotes/origin/main", "refs/remotes/origin/master", "refs/heads/main", "refs/heads/master"];
    let mut args = vec!["for-each-ref", "--format=%(refname)"];
    args.extend(names);
    let found = run(env, root, &args).await.ok()?;
    let found: Vec<&str> = found.lines().collect();
    names.iter().find(|n| found.contains(n)).map(|n| n.trim_start_matches("refs/remotes/").trim_start_matches("refs/heads/").to_string())
}

/// Working trees (`git worktree list`) and revisions of a project.
pub async fn sources(app: &App, project: &Project) -> Result<CodeSources> {
    let root = Path::new(&project.path);
    let main = Worktree { path: project.path.clone(), branch: None, main: true };
    if crate::git::git_dir(root).is_none() {
        return Ok(CodeSources { git: false, default_branch: None, worktrees: vec![main], refs: vec![] });
    }
    let env = &app.env;
    let list = run(env, root, &["worktree", "list", "--porcelain"]).await.unwrap_or_default();
    let mut worktrees: Vec<Worktree> = Vec::new();
    for block in list.split("\n\n") {
        let mut path = None;
        let mut branch = None;
        for line in block.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                path = Some(p.to_string());
            } else if let Some(b) = line.strip_prefix("branch ") {
                branch = Some(b.trim_start_matches("refs/heads/").to_string());
            }
        }
        let Some(path) = path.filter(|p| Path::new(p).is_dir()) else { continue };
        let path = std::fs::canonicalize(&path).map(|p| p.to_string_lossy().into_owned()).unwrap_or(path);
        let is_main = path == project.path;
        worktrees.push(Worktree { path, branch, main: is_main });
    }
    if !worktrees.iter().any(|w| w.main) {
        worktrees.insert(0, main);
    }
    worktrees.sort_by_key(|w| !w.main);

    let refs = run(
        env,
        root,
        &["for-each-ref", "--sort=-creatordate", "--format=%(refname)%00%(refname:short)%00%(creatordate:unix)", "refs/heads", "refs/remotes", "refs/tags", "refs/launchpad/pr"],
    )
    .await
    .unwrap_or_default();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut out = Vec::new();
    for line in refs.lines() {
        let mut f = line.split('\0');
        let (Some(full), Some(short), date) = (f.next(), f.next(), f.next()) else { continue };
        let date = date.and_then(|d| d.parse::<i64>().ok()).unwrap_or(0) * 1000;
        let (kind, name, label) = if let Some(n) = full.strip_prefix("refs/launchpad/pr/") {
            ("pr", full.to_string(), format!("#{n}"))
        } else if full.starts_with("refs/heads/") {
            ("branch", short.to_string(), short.to_string())
        } else if full.starts_with("refs/tags/") {
            ("tag", short.to_string(), short.to_string())
        } else if full.ends_with("/HEAD") {
            continue;
        } else {
            ("remote", short.to_string(), short.to_string())
        };
        let n = counts.entry(kind).or_default();
        *n += 1;
        if *n > 150 {
            continue;
        }
        out.push(RefInfo { name, kind: kind.into(), label, date });
    }
    Ok(CodeSources { git: true, default_branch: default_branch(env, root).await, worktrees, refs: out })
}

// ---------------------------------------------------------------- files

/// The merge base with the default branch, as `(label, commit)`. A working tree without a default
/// branch compares with `HEAD`, so its uncommitted changes still show.
async fn base(env: &UserEnv, src: &Source) -> Option<(String, String)> {
    let head = src.rev.as_ref().map(|(_, sha)| sha.as_str()).unwrap_or("HEAD");
    if let Some(default) = default_branch(env, &src.root).await
        && let Ok(commit) = run(env, &src.root, &["merge-base", &default, head]).await
    {
        return Some((default, commit));
    }
    if src.rev.is_none() {
        return run(env, &src.root, &["rev-parse", "--verify", "--quiet", "HEAD"]).await.ok().map(|c| ("HEAD".to_string(), c));
    }
    None
}

/// Every file path of a source, with change letters.
pub async fn files(app: &App, src: &Source) -> Result<FileList> {
    let env = &app.env;
    if !src.git {
        let root = src.root.clone();
        let files = tokio::task::spawn_blocking(move || walk(&root)).await?;
        return Ok(FileList { git: false, files, changes: BTreeMap::new(), base: None });
    }
    let root = &src.root;
    let mut changes = BTreeMap::new();
    let base = base(env, src).await;
    let mut files = match &src.rev {
        Some((_, sha)) => split_z(&run_raw(env, root, &["ls-tree", "-r", "--name-only", "-z", sha]).await?),
        None => {
            let mut files = split_z(&run_raw(env, root, &["ls-files", "--cached", "--deduplicate", "-z"]).await?);
            // Nested repositories and worktrees are listed as `dir/`: they are not files.
            for f in split_z(&run_raw(env, root, &["ls-files", "--others", "--exclude-standard", "-z"]).await?).into_iter().filter(|f| !f.ends_with('/')) {
                changes.insert(f.clone(), "U".to_string());
                files.push(f);
            }
            files
        }
    };
    if let Some((_, commit)) = &base {
        let mut args = vec!["diff", "--name-status", "--no-renames", "-z", commit.as_str()];
        if let Some((_, sha)) = &src.rev {
            args.push(sha);
        }
        let out = split_z(&run_raw(env, root, &args).await.unwrap_or_default());
        let mut deleted = std::collections::HashSet::new();
        for pair in out.chunks(2) {
            let [status, path] = pair else { continue };
            match status.chars().next() {
                Some('D') => {
                    deleted.insert(path.clone());
                }
                Some('A') => {
                    changes.insert(path.clone(), "A".into());
                }
                Some(_) => {
                    changes.insert(path.clone(), "M".into());
                }
                None => {}
            }
        }
        // Deleted in the working tree, but still in the index.
        if !deleted.is_empty() {
            files.retain(|f| !deleted.contains(f));
        }
    }
    files.sort();
    Ok(FileList { git: true, files, changes, base: base.map(|(label, _)| label) })
}

/// A folder that is not a git repository: hidden and ignored files are skipped.
fn walk(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for entry in ignore::WalkBuilder::new(root).require_git(false).build().flatten() {
        if entry.file_type().is_some_and(|t| t.is_file())
            && let Ok(rel) = entry.path().strip_prefix(root)
        {
            out.push(rel.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/"));
            if out.len() >= 200_000 {
                break;
            }
        }
    }
    out.sort();
    out
}

const IMAGES: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "ico", "avif"];

fn extension(path: &str) -> String {
    Path::new(path).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

pub fn is_image(path: &str) -> bool {
    IMAGES.contains(&extension(path).as_str())
}

/// Bytes of a file, at most `limit`. `Ok(Err(size))` when it is larger.
async fn read(env: &UserEnv, src: &Source, path: &str, limit: u64) -> Result<std::result::Result<Vec<u8>, u64>> {
    let rel = check_path(path)?;
    match &src.rev {
        Some((_, sha)) => {
            let object = format!("{sha}:{path}");
            let size: u64 = run(env, &src.root, &["cat-file", "-s", &object]).await.with_context(|| format!("{path} does not exist here"))?.parse()?;
            if size > limit {
                return Ok(Err(size));
            }
            Ok(Ok(run_raw(env, &src.root, &["cat-file", "blob", &object]).await?))
        }
        None => {
            let full = inside(&src.root, rel)?;
            let meta = std::fs::metadata(&full)?;
            if meta.is_dir() {
                bail!("{path} is a folder (a git submodule?)");
            }
            if meta.len() > limit {
                return Ok(Err(meta.len()));
            }
            Ok(Ok(tokio::fs::read(&full).await?))
        }
    }
}

/// Raw bytes, for images.
pub async fn raw(app: &App, src: &Source, path: &str) -> Result<Vec<u8>> {
    read(&app.env, src, path, 4 * MAX_LOAD).await?.map_err(|size| anyhow::anyhow!("{path} is too large ({size} bytes)"))
}

pub async fn file(app: &App, src: &Source, path: &str) -> Result<CodeFile> {
    let env = &app.env;
    let (language, language_name) = language(path).map(|(id, name)| (Some(id.to_string()), Some(name.to_string()))).unwrap_or_default();
    let mut f = CodeFile {
        path: path.to_string(),
        size: 0,
        kind: "text".into(),
        language,
        language_name,
        text: None,
        marks: vec![],
        added: 0,
        removed: 0,
        letter: None,
        base: None,
        dirty: false,
    };
    let bytes = match read(env, src, path, MAX_LOAD).await? {
        Ok(b) => b,
        Err(size) => {
            f.size = size;
            f.kind = "too_large".into();
            return Ok(f);
        }
    };
    f.size = bytes.len() as u64;
    if is_image(path) {
        f.kind = "image".into();
        return Ok(f);
    }
    // Like git: a NUL byte in the first 8 KB means binary.
    if bytes[..bytes.len().min(8000)].contains(&0) {
        f.kind = "binary".into();
        return Ok(f);
    }
    let text = String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
    if src.git {
        let lines = text.lines().count() as u32;
        let _ = changed_lines(env, src, path, lines, &mut f).await;
    }
    f.text = Some(text);
    Ok(f)
}

/// Bars next to the line numbers: `git diff -U0 <base>` for this file. An untracked file is all new.
async fn changed_lines(env: &UserEnv, src: &Source, path: &str, lines: u32, f: &mut CodeFile) -> Result<()> {
    let root = &src.root;
    if src.rev.is_none() {
        f.dirty = !run_raw(env, root, &["diff", "--name-only", "HEAD", "--", path]).await.unwrap_or_default().is_empty();
    }
    if let Some((label, commit)) = base(env, src).await {
        f.base = Some(label);
        let mut args = vec!["diff", "-U0", "--no-color", "--no-ext-diff", commit.as_str()];
        if let Some((_, sha)) = &src.rev {
            args.push(sha);
        }
        args.extend(["--", path]);
        let diff = String::from_utf8_lossy(&run_raw(env, root, &args).await?).into_owned();
        if !diff.trim().is_empty() {
            let (marks, added, removed) = line_marks(&diff);
            f.letter = Some(if diff.lines().any(|l| l.starts_with("new file mode")) { "A" } else { "M" }.into());
            f.marks = marks;
            f.added = added;
            f.removed = removed;
            return Ok(());
        }
    }
    // Untracked (also every file of a repository without commits): all lines are new.
    if src.rev.is_none() && !run_raw(env, root, &["ls-files", "--others", "--exclude-standard", "-z", "--", path]).await?.is_empty() {
        f.letter = Some("U".into());
        f.dirty = true;
        f.added = lines;
        if lines > 0 {
            f.marks = vec![LineMark { start: 1, end: lines, kind: "add".into() }];
        }
    }
    Ok(())
}

/// Line marks from the hunk headers of `git diff -U0` (`@@ -a,b +c,d @@`). In a hunk, the first
/// min(b, d) new lines are modified and the others are added; a hunk without new lines marks a
/// deletion after line c. Returns the marks and the added and removed line counts.
pub fn line_marks(diff: &str) -> (Vec<LineMark>, u32, u32) {
    let (mut marks, mut added, mut removed) = (Vec::new(), 0, 0);
    for line in diff.lines() {
        let Some(rest) = line.strip_prefix("@@ -") else { continue };
        let Some((ranges, _)) = rest.split_once(" @@") else { continue };
        let Some((old, new)) = ranges.split_once(" +") else { continue };
        let range = |r: &str| -> Option<(u32, u32)> {
            match r.split_once(',') {
                Some((s, n)) => Some((s.parse().ok()?, n.parse().ok()?)),
                None => Some((r.parse().ok()?, 1)),
            }
        };
        let (Some((_, del)), Some((start, add))) = (range(old), range(new)) else { continue };
        removed += del;
        added += add;
        if add == 0 {
            marks.push(LineMark { start: start.max(1), end: start.max(1), kind: "del".into() });
            continue;
        }
        let modified = del.min(add);
        if modified > 0 {
            marks.push(LineMark { start, end: start + modified - 1, kind: "mod".into() });
        }
        if add > modified {
            marks.push(LineMark { start: start + modified, end: start + add - 1, kind: "add".into() });
        }
    }
    (marks, added, removed)
}

/// Shiki language id and display name, from the file name.
pub fn language(path: &str) -> Option<(&'static str, &'static str)> {
    let name = Path::new(path).file_name()?.to_string_lossy().to_ascii_lowercase();
    let by_name = match name.as_str() {
        "dockerfile" | "containerfile" => Some(("docker", "Dockerfile")),
        "makefile" | "gnumakefile" => Some(("make", "Makefile")),
        "cmakelists.txt" => Some(("cmake", "CMake")),
        "cargo.lock" | "poetry.lock" | "uv.lock" | "pipfile" => Some(("toml", "TOML")),
        ".bashrc" | ".zshrc" | ".profile" | ".bash_profile" | ".envrc" => Some(("shellscript", "Shell")),
        ".gitignore" | ".dockerignore" | ".gitattributes" | ".editorconfig" | ".npmrc" => Some(("ini", "Config")),
        _ => None,
    };
    if by_name.is_some() {
        return by_name;
    }
    Some(match extension(path).as_str() {
        "rs" => ("rust", "Rust"),
        "py" | "pyi" | "pyw" => ("python", "Python"),
        "ipynb" => ("json", "Jupyter notebook"),
        "ts" | "mts" | "cts" => ("typescript", "TypeScript"),
        "tsx" => ("tsx", "TSX"),
        "js" | "mjs" | "cjs" => ("javascript", "JavaScript"),
        "jsx" => ("jsx", "JSX"),
        "json" | "jsonl" | "webmanifest" => ("json", "JSON"),
        "jsonc" => ("jsonc", "JSON with comments"),
        "toml" => ("toml", "TOML"),
        "yaml" | "yml" => ("yaml", "YAML"),
        "md" | "markdown" => ("markdown", "Markdown"),
        "mdx" => ("mdx", "MDX"),
        "html" | "htm" => ("html", "HTML"),
        "css" => ("css", "CSS"),
        "scss" => ("scss", "SCSS"),
        "less" => ("less", "Less"),
        "sh" | "bash" | "zsh" => ("shellscript", "Shell"),
        "fish" => ("fish", "Fish"),
        "go" => ("go", "Go"),
        "java" => ("java", "Java"),
        "kt" | "kts" => ("kotlin", "Kotlin"),
        "swift" => ("swift", "Swift"),
        "c" | "h" => ("c", "C"),
        "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" | "cu" | "cuh" => ("cpp", "C++"),
        "cs" => ("csharp", "C#"),
        "rb" => ("ruby", "Ruby"),
        "php" => ("php", "PHP"),
        "sql" => ("sql", "SQL"),
        "lua" => ("lua", "Lua"),
        "xml" | "plist" | "xsd" => ("xml", "XML"),
        "svg" => ("xml", "SVG"),
        "vue" => ("vue", "Vue"),
        "svelte" => ("svelte", "Svelte"),
        "ini" | "cfg" | "conf" | "properties" => ("ini", "INI"),
        "proto" => ("proto", "Protocol Buffers"),
        "graphql" | "gql" => ("graphql", "GraphQL"),
        "r" => ("r", "R"),
        "scala" => ("scala", "Scala"),
        "dart" => ("dart", "Dart"),
        "zig" => ("zig", "Zig"),
        "nix" => ("nix", "Nix"),
        "tf" | "tfvars" | "hcl" => ("hcl", "HCL"),
        "diff" | "patch" => ("diff", "Diff"),
        "jl" => ("julia", "Julia"),
        "ex" | "exs" => ("elixir", "Elixir"),
        "hs" => ("haskell", "Haskell"),
        "ml" | "mli" => ("ocaml", "OCaml"),
        "ps1" => ("powershell", "PowerShell"),
        "bat" | "cmd" => ("bat", "Batch"),
        "tex" => ("latex", "LaTeX"),
        "rst" => ("rst", "reStructuredText"),
        "csv" => ("csv", "CSV"),
        "service" | "desktop" | "timer" => ("ini", "INI"),
        _ => return None,
    })
}

// ---------------------------------------------------------------- search

/// `git grep` on the source: fixed strings unless `regex`, case-insensitive unless `case`. At most
/// 1,000 results. Dropping the future (a new query, a closed request) kills git.
pub async fn search(app: &App, src: &Source, q: &SearchQuery) -> Result<SearchResult> {
    if q.q.is_empty() {
        return Ok(SearchResult { hits: vec![], truncated: false });
    }
    let mut cmd = crate::procs::command(&app.env, "git", Some(&src.root));
    cmd.args(["-c", "submodule.recurse=false", "-c", "core.quotepath=off", "grep", "-n", "--column", "-I", "-z", "--no-color"]);
    cmd.arg(if q.regex { "-E" } else { "-F" });
    if !q.case {
        cmd.arg("-i");
    }
    if q.word {
        cmd.arg("-w");
    }
    if !src.git {
        cmd.args(["--no-index", "--exclude-standard"]);
    } else if src.rev.is_none() {
        cmd.arg("--untracked");
    }
    cmd.arg("-e").arg(&q.q);
    if let Some((_, sha)) = &src.rev {
        cmd.arg(sha);
    }
    cmd.arg("--");
    let mut child = cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn()?;
    let prefix = src.rev.as_ref().map(|(_, sha)| format!("{sha}:"));
    let mut reader = tokio::io::BufReader::new(child.stdout.take().context("git stdout")?);
    let mut hits = Vec::new();
    let mut truncated = false;
    let mut buf = Vec::new();
    let read = async {
        while reader.read_until(b'\n', &mut buf).await? > 0 {
            if let Some(hit) = parse_hit(&buf, prefix.as_deref()) {
                if hits.len() == MAX_HITS {
                    truncated = true;
                    break;
                }
                hits.push(hit);
            }
            buf.clear();
        }
        anyhow::Ok(())
    };
    tokio::time::timeout(std::time::Duration::from_secs(30), read).await.context("the search took more than 30 s")??;
    if truncated {
        let _ = child.kill().await;
    } else {
        let out = child.wait_with_output().await?;
        // Exit code 1: no match.
        if !out.status.success() && out.status.code() != Some(1) {
            bail!("{}", String::from_utf8_lossy(&out.stderr).trim().trim_start_matches("fatal: "));
        }
    }
    Ok(SearchResult { hits, truncated })
}

/// `path\0line\0column\0text` (with `-z`). Long lines are cut around the match.
fn parse_hit(line: &[u8], prefix: Option<&str>) -> Option<SearchHit> {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    let mut parts = line.splitn(4, |b| *b == 0);
    let path = String::from_utf8_lossy(parts.next()?).into_owned();
    let path = match prefix {
        Some(p) => path.strip_prefix(p)?.to_string(),
        None => path,
    };
    let number: u32 = std::str::from_utf8(parts.next()?).ok()?.parse().ok()?;
    let column: usize = std::str::from_utf8(parts.next()?).ok()?.parse().ok()?;
    let text = parts.next().unwrap_or_default();
    let text = if text.len() > 400 {
        let start = column.saturating_sub(120).min(text.len());
        let end = (start + 400).min(text.len());
        let cut = String::from_utf8_lossy(&text[start..end]).into_owned();
        format!("{}{cut}{}", if start > 0 { "…" } else { "" }, if end < text.len() { "…" } else { "" })
    } else {
        String::from_utf8_lossy(text).into_owned()
    };
    Some(SearchHit { path, line: number, text })
}

// ---------------------------------------------------------------- commit

/// The commit shown, and the GitHub repository of a remote branch that contains it.
pub async fn commit(app: &App, src: &Source) -> Result<CommitInfo> {
    if !src.git {
        return Ok(CommitInfo { sha: None, github: None });
    }
    let env = &app.env;
    let sha = match &src.rev {
        Some((_, sha)) => sha.clone(),
        None => match run(env, &src.root, &["rev-parse", "--verify", "--quiet", "HEAD"]).await {
            Ok(s) if !s.is_empty() => s,
            _ => return Ok(CommitInfo { sha: None, github: None }),
        },
    };
    let branches = run(env, &src.root, &["branch", "-r", "--format=%(refname:short)", "--contains", &sha]).await.unwrap_or_default();
    let remotes = crate::git::remotes(&src.root);
    let mut names: Vec<&str> = branches.lines().filter_map(|b| b.split('/').next()).collect();
    names.sort_by_key(|n| *n != "origin");
    let github = names.iter().find_map(|n| remotes.iter().find(|(r, _)| r == n).and_then(|(_, url)| crate::git::github_repo(url)));
    Ok(CommitInfo { sha: Some(sha), github })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn commit_ids_ignore_missing_objects_blobs_tags_and_hash_named_refs() {
        let root = std::env::temp_dir().join(format!("launchpad-commit-links-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let env = UserEnv::resolve().await;
        run(&env, &root, &["init"]).await.unwrap();
        run(&env, &root, &["-c", "user.name=Test", "-c", "user.email=test@example.invalid", "-c", "commit.gpgsign=false", "commit", "--allow-empty", "-m", "test"]).await.unwrap();
        let sha = run(&env, &root, &["rev-parse", "HEAD"]).await.unwrap();
        std::fs::write(root.join("blob"), "not a commit").unwrap();
        let blob = run(&env, &root, &["hash-object", "-w", "blob"]).await.unwrap();
        run(&env, &root, &["branch", "deadbee", "HEAD"]).await.unwrap();
        run(&env, &root, &["-c", "user.name=Test", "-c", "user.email=test@example.invalid", "-c", "tag.gpgsign=false", "tag", "-a", "test-tag", "-m", "tag"]).await.unwrap();
        let tag = run(&env, &root, &["rev-parse", "test-tag"]).await.unwrap();
        let short = sha[..7].to_string();
        let upper = short.to_ascii_uppercase();
        let oids = vec![short.clone(), upper.clone(), sha.clone(), blob, tag, "deadbee".into()];
        let found = resolve_commit_ids(&env, &root, &oids).await.unwrap();
        assert_eq!(found, BTreeMap::from([(short, sha.clone()), (upper, sha.clone()), (sha.clone(), sha.clone())]));
        // A second loose object with the same prefix makes abbreviations ambiguous.
        let other = format!("{}{}", &sha[..39], if sha.ends_with('0') { '1' } else { '0' });
        let object = root.join(".git/objects").join(&sha[..2]).join(&sha[2..]);
        let collision = root.join(".git/objects").join(&other[..2]).join(&other[2..]);
        std::fs::copy(object, collision).unwrap();
        assert!(resolve_commit_ids(&env, &root, &[sha[..7].into()]).await.unwrap().is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn commit_diffs_cover_root_renames_deletions_binary_empty_and_first_parent_merges() {
        let root = std::env::temp_dir().join(format!("launchpad-commit-diff-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let env = UserEnv::resolve().await;
        run(&env, &root, &["init"]).await.unwrap();
        for (key, value) in [("user.name", "Test"), ("user.email", "test@example.invalid"), ("commit.gpgsign", "false"), ("core.filemode", "false")] {
            run(&env, &root, &["config", key, value]).await.unwrap();
        }
        for (name, text) in [("old name.txt", "one\ntwo\nthree\n"), ("gone.txt", "gone\n"), ("mode.txt", "mode\n")] {
            std::fs::write(root.join(name), text).unwrap();
        }
        std::fs::write(root.join("binary.dat"), [0, 1, 2]).unwrap();
        run(&env, &root, &["add", "--all"]).await.unwrap();
        run(&env, &root, &["commit", "-m", "root changes"]).await.unwrap();
        let first = run(&env, &root, &["rev-parse", "HEAD"]).await.unwrap();
        let summary = commit_diff_at(&env, &root, &first).await.unwrap();
        assert!(summary["parent"].is_null());
        assert_eq!(summary["files"].as_array().unwrap().len(), 4);
        assert!(summary["files"].as_array().unwrap().iter().all(|f| f["status"] == "added" && f.get("patch").is_none()));
        let root_patch = commit_diff_file_at(&env, &root, &first, "old name.txt").await.unwrap();
        assert!(root_patch["patch"].as_str().unwrap().contains("+one"));
        std::fs::rename(root.join("old name.txt"), root.join("new name.txt")).unwrap();
        std::fs::write(root.join("new name.txt"), "one\ntwo\nthree\nfour\n").unwrap();
        std::fs::remove_file(root.join("gone.txt")).unwrap();
        std::fs::write(root.join("binary.dat"), [0, 3, 4]).unwrap();
        std::fs::write(root.join("[literal].txt"), "literal\n").unwrap();
        run(&env, &root, &["add", "--all"]).await.unwrap();
        run(&env, &root, &["update-index", "--chmod=+x", "mode.txt"]).await.unwrap();
        run(&env, &root, &["commit", "-m", "second changes"]).await.unwrap();
        let second = run(&env, &root, &["rev-parse", "HEAD"]).await.unwrap();
        let summary = commit_diff_at(&env, &root, &second).await.unwrap();
        assert_eq!(summary["parent"], first);
        let files = summary["files"].as_array().unwrap();
        let renamed = files.iter().find(|f| f["filename"] == "new name.txt").unwrap();
        assert_eq!(renamed["status"], "renamed");
        assert_eq!(renamed["previous_filename"], "old name.txt");
        assert_eq!(renamed["additions"], 1);
        assert_eq!(renamed["deletions"], 0);
        let patch = commit_diff_file_at(&env, &root, &second, "new name.txt").await.unwrap();
        assert!(patch["patch"].as_str().unwrap().contains("+four"));
        assert!(commit_diff_file_at(&env, &root, &second, "gone.txt").await.unwrap()["patch"].as_str().unwrap().contains("-gone"));
        assert_eq!(commit_diff_file_at(&env, &root, &second, "binary.dat").await.unwrap()["patch"], "");
        assert_eq!(commit_diff_file_at(&env, &root, &second, "mode.txt").await.unwrap()["patch"], "");
        assert!(commit_diff_file_at(&env, &root, &second, "[literal].txt").await.unwrap()["patch"].as_str().unwrap().contains("+literal"));
        assert!(commit_diff_file_at(&env, &root, &second, "unlisted.txt").await.is_err());
        run(&env, &root, &["commit", "--allow-empty", "--allow-empty-message", "-m", ""]).await.unwrap();
        let empty = run(&env, &root, &["rev-parse", "HEAD"]).await.unwrap();
        let empty_diff = commit_diff_at(&env, &root, &empty).await.unwrap();
        assert!(empty_diff["files"].as_array().unwrap().is_empty());
        assert_eq!(empty_diff["subject"], "");
        assert_eq!(empty_diff["parent"], second);
        let branch = run(&env, &root, &["symbolic-ref", "--short", "HEAD"]).await.unwrap();
        run(&env, &root, &["checkout", "-b", "side", &first]).await.unwrap();
        std::fs::write(root.join("side.txt"), "side\n").unwrap();
        run(&env, &root, &["add", "side.txt"]).await.unwrap();
        run(&env, &root, &["commit", "-m", "side"]).await.unwrap();
        run(&env, &root, &["checkout", &branch]).await.unwrap();
        run(&env, &root, &["merge", "--no-ff", "side", "-m", "merge"]).await.unwrap();
        let merge = run(&env, &root, &["rev-parse", "HEAD"]).await.unwrap();
        let summary = commit_diff_at(&env, &root, &merge).await.unwrap();
        assert_eq!(summary["parent"], empty);
        assert_eq!(summary["files"].as_array().unwrap().len(), 1);
        assert_eq!(summary["files"][0]["filename"], "side.txt");
        assert!(commit_diff_file_at(&env, &root, &merge, "side.txt").await.unwrap()["patch"].as_str().unwrap().contains("+side"));
        std::fs::remove_dir_all(root).unwrap();
    }

    fn mark(start: u32, end: u32, kind: &str) -> LineMark {
        LineMark { start, end, kind: kind.into() }
    }

    #[test]
    fn marks_from_hunks() {
        let diff = "diff --git a/x.py b/x.py
--- a/x.py
+++ b/x.py
@@ -10,2 +10,4 @@ def glob(self):
-a
-b
+a2
+b2
+c
+d
@@ -30 +32 @@
-x
+y
@@ -40,3 +41,0 @@ class X:
-p
-q
-r
@@ -0,0 +1,2 @@
+new
+lines
@@ -50,0 +52 @@
+one
";
        let (marks, added, removed) = line_marks(diff);
        assert_eq!(marks, vec![mark(10, 11, "mod"), mark(12, 13, "add"), mark(32, 32, "mod"), mark(41, 41, "del"), mark(1, 2, "add"), mark(52, 52, "add")]);
        assert_eq!((added, removed), (8, 6));
        assert_eq!(line_marks("@@ -1,2 +0,0 @@\n-a\n-b\n").0, vec![mark(1, 1, "del")]);
    }

    #[test]
    fn paths_stay_relative() {
        assert!(check_path("src/x.py").is_ok());
        for bad in ["", "/etc/passwd", "../x", "src/../../x", "./x", "a\0b"] {
            assert!(check_path(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn grep_lines() {
        let hit = parse_hit(b"abc:src/a:b.rs\x0012\x005\x00let x = 1;\n", Some("abc:")).unwrap();
        assert_eq!((hit.path.as_str(), hit.line, hit.text.as_str()), ("src/a:b.rs", 12, "let x = 1;"));
        assert!(parse_hit(b"src/a.rs\x00x\x001\x00t", None).is_none());
    }
}

//! The `funes` helper program (huggingface/funes, pinned in Home's image): where it is, the environment it runs in
//! and its output.
//!
//! Home runs it as a one-shot child per call, never as a service: `funes index <turns file>` to add turns and
//! `funes recall` to search. Only these two: `funes status` asks the network for a newer release. Each workspace has its own `FUNES_HOME`, so the workspace boundary is a folder, not a
//! filter. The child gets a cleared environment: no Hugging Face token, its own `HOME`, the model cache of the image
//! (`HF_HOME`, `HF_HUB_CACHE`, offline) and no telemetry (`DO_NOT_TRACK`). Funes' own integrations, hooks and Hub
//! push are never used.
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use launchpad_core::App;

/// The models Funes loads: the embedder (indexing and recall) and the reranker (recall).
pub const EMBEDDER: &str = "BAAI/bge-small-en-v1.5";
pub const RERANKER: &str = "Alibaba-NLP/gte-reranker-modernbert-base";
const MODEL_FILES: [&str; 2] = ["tokenizer.json", "model.safetensors"];

/// Exit status of a Funes writer that found the memory locked by another one (sysexits' EX_TEMPFAIL).
pub const BUSY: i32 = 75;

/// `$LAUNCHPAD_FUNES`, else `funes` on PATH (Home's image installs it).
pub fn binary(app: &App) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("LAUNCHPAD_FUNES").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p)).filter(|p| p.is_file());
    }
    let paths = [app.env.get("PATH").map(str::to_string), std::env::var("PATH").ok(), Some("/usr/local/bin:/usr/bin:/bin".to_string())];
    paths.into_iter().flatten().flat_map(|path| path.split(':').filter(|d| !d.is_empty()).map(|d| PathBuf::from(d).join("funes")).collect::<Vec<_>>()).find(|p| p.is_file())
}

/// Every workspace's memory, and Funes' own `HOME`: `<data>/recall`.
pub fn root(app: &App) -> PathBuf {
    app.paths.data_dir.join("recall")
}

/// The Hugging Face home with the model weights: `$LAUNCHPAD_FUNES_HF_HOME` (Home's image bakes them in at fixed
/// revisions), else `<data>/recall/hf` (a dev Home: put the snapshots there).
pub fn hf_home(app: &App) -> PathBuf {
    std::env::var_os("LAUNCHPAD_FUNES_HF_HOME").filter(|p| !p.is_empty()).map(PathBuf::from).unwrap_or_else(|| root(app).join("hf"))
}

/// The snapshot of a model in the cache, as Funes finds it: any snapshot folder with every file it reads.
fn snapshot(hf_home: &Path, repo: &str) -> Option<PathBuf> {
    let snaps = hf_home.join("hub").join(format!("models--{}", repo.replace('/', "--"))).join("snapshots");
    std::fs::read_dir(snaps).ok()?.flatten().map(|e| e.path()).find(|p| MODEL_FILES.iter().all(|f| p.join(f).exists()))
}

/// The revision of a model in the cache (its snapshot folder's name); None when its weights are missing. Funes
/// would download a missing model: Home never lets it (`ready` refuses first, and the child is offline).
pub fn model_revision(app: &App, repo: &str) -> Option<String> {
    snapshot(&hf_home(app), repo).and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
}

/// What a call needs: the program, and the embedder's weights (`rerank`: the reranker's too).
pub fn ready(app: &App, rerank: bool) -> Result<PathBuf> {
    let bin = binary(app).context("recall is not available: the funes program is not installed on Home (Home's image has it; on a dev Home put funes on PATH or set LAUNCHPAD_FUNES)")?;
    for repo in [Some(EMBEDDER), rerank.then_some(RERANKER)].into_iter().flatten() {
        if model_revision(app, repo).is_none() {
            bail!("recall is not available: the weights of {repo} are missing from {} (Home's image holds them; Home never downloads them)", hf_home(app).join("hub").display());
        }
    }
    Ok(bin)
}

pub struct Output {
    /// None: killed (the time limit).
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }
    /// The end of what it printed, for an error message.
    pub fn tail(&self) -> String {
        let text = if self.stderr.trim().is_empty() { &self.stdout } else { &self.stderr };
        let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        let tail = lines[lines.len().saturating_sub(4)..].join(" | ");
        tail.chars().take(600).collect()
    }
}

/// Runs `funes <args>` on the memory in `home` (its `FUNES_HOME`), killed after `limit`. `background`: at a low
/// priority (indexing), so it never competes with Home.
pub async fn run(app: &App, bin: &Path, home: &Path, args: &[String], limit: Duration, background: bool) -> Result<Output> {
    let root = root(app);
    tokio::fs::create_dir_all(home).await.with_context(|| format!("creating {}", home.display()))?;
    let hf = hf_home(app);
    let mut cmd = if background && Path::new("/usr/bin/nice").is_file() {
        let mut c = tokio::process::Command::new("/usr/bin/nice");
        c.arg("-n").arg("15").arg(bin);
        c
    } else {
        tokio::process::Command::new(bin)
    };
    cmd.args(args)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", &root)
        .env("FUNES_HOME", home)
        .env("HF_HOME", &hf)
        .env("HF_HUB_CACHE", hf.join("hub"))
        .env("HF_HUB_OFFLINE", "1")
        .env("HF_HUB_DISABLE_IMPLICIT_TOKEN", "1")
        .env("DO_NOT_TRACK", "1")
        .env("FUNES_NO_TELEMETRY", "1")
        .env("NO_COLOR", "1")
        .env("COLUMNS", "120")
        .current_dir(&root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = cmd.spawn().with_context(|| format!("starting {}", bin.display()))?;
    match tokio::time::timeout(limit, child.wait_with_output()).await {
        Ok(out) => {
            let out = out.context("waiting for funes")?;
            Ok(Output { code: out.status.code(), stdout: String::from_utf8_lossy(&out.stdout).into_owned(), stderr: String::from_utf8_lossy(&out.stderr).into_owned() })
        }
        // Dropping the child kills it.
        Err(_) => Ok(Output { code: None, stdout: String::new(), stderr: format!("no answer after {} s", limit.as_secs()) }),
    }
}

/// One hit of `funes recall`, as its agent format prints it (funes `src/ui/render.rs`, a published contract).
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub ts: String,
    pub session_id: String,
    /// The hit's turn: the session's Funes `seq`.
    pub fseq: i64,
    pub block_type: String,
    pub score: String,
    /// The passage (`--oneline`: its first words).
    pub text: String,
    /// The turns framing the hit's exchange: role, Funes `seq`, text.
    pub exchange: Vec<(String, i64, String)>,
}

/// The `→ get <session> --from a --to b` target of a hit: the session and the hit's own `seq` (the line reaches 3
/// turns each side, clamped at 0, so the hit is always 3 before `--to`).
fn target(s: &str) -> Option<(String, i64)> {
    let mut words = s.split_whitespace();
    let session = words.next()?.to_string();
    let (mut from, mut to) = (None, None);
    while let Some(w) = words.next() {
        match w {
            "--from" => from = words.next()?.parse::<i64>().ok(),
            "--to" => to = words.next()?.parse::<i64>().ok(),
            _ => {}
        }
    }
    from?;
    Some((session, to? - 3))
}

/// `[<ts>] <harness> <workdir>/<session8> <block_type>  score=<s.sss>`: the time, the block type and the score.
fn header(line: &str) -> Option<(String, String, String)> {
    let rest = line.strip_prefix('[')?;
    let (ts, rest) = rest.split_once("] ")?;
    let (left, score) = rest.rsplit_once("  score=")?;
    if score.is_empty() || !score.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-') {
        return None;
    }
    let block_type = left.rsplit(' ').next()?;
    matches!(block_type, "text" | "thinking" | "tool_use" | "tool_result").then(|| (ts.to_string(), block_type.to_string(), score.to_string()))
}

/// `  ↳ [<role> text seq<N>]`
fn frame(line: &str) -> Option<(String, i64)> {
    let inner = line.strip_prefix("  ↳ [")?.strip_suffix(']')?;
    let (role, seq) = inner.rsplit_once(" text seq")?;
    Some((role.to_string(), seq.parse().ok()?))
}

/// The hits of a `funes recall` reply (the full agent format).
pub fn parse_hits(out: &str) -> Vec<Hit> {
    let lines: Vec<&str> = out.lines().collect();
    let mut hits: Vec<Hit> = vec![];
    let mut i = 0;
    // Where the open hit's text goes: its passage, or the last turn of its exchange.
    while i < lines.len() {
        let line = lines[i];
        if let Some((ts, block_type, score)) = header(line)
            && let Some((session_id, fseq)) = lines.get(i + 1).and_then(|l| l.strip_prefix("  → get ")).and_then(target)
        {
            hits.push(Hit { ts, session_id, fseq, block_type, score, text: String::new(), exchange: vec![] });
            i += 2;
            continue;
        }
        if let Some(hit) = hits.last_mut() {
            if let Some((role, seq)) = frame(line) {
                hit.exchange.push((role, seq, String::new()));
            } else {
                let into = match hit.exchange.last_mut() {
                    Some((_, _, text)) => text,
                    None => &mut hit.text,
                };
                if !into.is_empty() {
                    into.push('\n');
                }
                into.push_str(line);
            }
        }
        i += 1;
    }
    // Each hit ends with a `---` line; the reply ends with Funes' own advice after the last one.
    for hit in &mut hits {
        let last = match hit.exchange.last_mut() {
            Some((_, _, text)) => text,
            None => &mut hit.text,
        };
        if let Some(at) = last.rfind("\n---") {
            last.truncate(at);
        } else if last.starts_with("---") {
            last.clear();
        }
    }
    hits
}

/// The hits of a `funes recall --oneline` reply: one row each.
pub fn parse_oneline(out: &str) -> Vec<Hit> {
    out.lines()
        .filter_map(|line| {
            let (head, rest) = line.split_once("  → get ")?;
            let (ts, block_type, score) = header(head)?;
            let (get, text) = rest.split_once(" · ").unwrap_or((rest, ""));
            let (session_id, fseq) = target(get)?;
            Some(Hit { ts, session_id, fseq, block_type, score, text: text.to_string(), exchange: vec![] })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hits_are_read_from_the_agent_format() {
        let out = "[2026-06-19T01:29:59Z] claude /0123abcd tool_result  score=0.578\n  → get 0123abcd-ef --from 4 --to 10\n[tool_result bash] the build\n---\nfailed\n  ↳ [user text seq3]\nwhy does it fail?\n  ↳ [assistant text seq9]\nthe target is wrong\n---\n[2026-06-20T00:00:00.000Z] codex -/9999aaaa text  score=0.100\n  → get 9999aaaa --from 0 --to 4\nsecond\n---\nif these miss: rephrase in the words the hits use, then raise --k; a get line only reads around its own hit\n";
        let hits = parse_hits(out);
        assert_eq!(hits.len(), 2);
        assert_eq!((hits[0].session_id.as_str(), hits[0].fseq, hits[0].block_type.as_str(), hits[0].score.as_str()), ("0123abcd-ef", 7, "tool_result", "0.578"));
        // A `---` inside the passage stays; the one that closes the hit goes.
        assert_eq!(hits[0].text, "[tool_result bash] the build\n---\nfailed");
        assert_eq!(hits[0].exchange, vec![("user".to_string(), 3, "why does it fail?".to_string()), ("assistant".to_string(), 9, "the target is wrong".to_string())]);
        // A hit near the session's start: `--from` is clamped at 0, the hit is still 3 before `--to`.
        assert_eq!((hits[1].fseq, hits[1].text.as_str()), (1, "second"));
        assert!(parse_hits("no results\nrephrase in other words").is_empty());
    }

    #[test]
    fn rows_are_read_from_the_oneline_format() {
        let out = "[2026-06-19T01:29:59.000Z] claude -/01234567 text  score=0.578  → get 0123456789abcdef --from 4 --to 10 · the decision was made\nif these miss: rephrase\n";
        let hits = parse_oneline(out);
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].session_id.as_str(), hits[0].fseq, hits[0].text.as_str()), ("0123456789abcdef", 7, "the decision was made"));
    }
}

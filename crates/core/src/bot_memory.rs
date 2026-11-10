//! A bot's memory: the notes in `memory/` of its store (`bot-<id>`), next to `BOT.md` and `bot.toml`.
//!
//! - Home writes them like the knowledge base (`kb::apply_to`: one writer at a time, author and line counts in
//!   the history, the content the writer started from; a concurrent change keeps both versions). A session
//!   writes only its own bot's memory, which Home takes from its own session record, never from the runner.
//! - Runners mirror the whole store read-only into a bot's sessions (`$LAUNCHPAD_BOT`): `BOT.md`, `bot.toml` and
//!   `memory/`. Agents write with `launchpad bot memory write|append|rm`.
//! - Notes kept in the knowledge base's `bots/<slug>/` before bots had their own memory move here once
//!   (`move_from_kb`).

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::App;
use crate::kb::{self, Author, LocalOp, Op};
use crate::plain;

/// The folder of the notes in a bot's store.
pub const DIR: &str = "memory/";

/// A path given to `launchpad bot memory`, as a path of the store: relative to `memory/` (a leading `memory/` is
/// accepted too).
pub fn store_path(path: &str) -> Result<String> {
    let p = path.trim().trim_start_matches("./");
    let p = p.strip_prefix(DIR).unwrap_or(p);
    plain::check_path(p)?;
    Ok(format!("{DIR}{p}"))
}

/// Home: writes to a bot's memory. Every path must be under `memory/`.
pub(crate) async fn apply(app: &App, bot_id: &str, author: Author, ops: Vec<Op>) -> Result<Vec<String>> {
    let bot = crate::bots::get(app, bot_id).await?;
    for op in &ops {
        let path = match op {
            Op::Write { path, .. } | Op::Append { path, .. } | Op::Remove { path } => path,
        };
        ensure!(path.starts_with(DIR), "{path}: a bot writes only its memory/ folder");
    }
    let name = format!("{}'s memory", bot.settings.name);
    let place = kb::Place { store: crate::bots::store(bot_id), name: &name, rm: "launchpad bot memory rm" };
    let lines = kb::apply_to(app, &place, author, ops).await?;
    app.bus.publish("bots", "changed", json!({"id": bot_id}));
    Ok(lines)
}

/// Home: writes a runner sends for one of its bot sessions (`launchpad bot memory` through the tunnel). The bot is
/// the one Home's record of the session names.
pub async fn runner_apply(app: &App, runner_id: &str, body: Value) -> Result<Value> {
    let ops: Vec<Op> = serde_json::from_value(body["ops"].clone())?;
    let session_id = body["session_id"].as_str().context("session_id")?;
    crate::home_git::check_session(app, runner_id, session_id)?;
    let s = app.sessions.get(session_id).context("no such session")?;
    let bot = s.launch_spec.bot.as_ref().context("this session is not a bot's: it has no bot memory")?;
    let b = crate::bots::get(app, &bot.id).await.context("this session's bot no longer exists")?;
    ensure!(b.settings.launchpad != crate::vault::LaunchpadAccess::Read, "{}'s Launchpad CLI access is read: it reads its memory but cannot write it", b.settings.name);
    let author = kb::agent_author(app, session_id).await?;
    Ok(json!({"lines": apply(app, &bot.id, author, ops).await?}))
}

// ---------------------------------------------------------------- the move out of the knowledge base

/// Whether an INDEX.md line links to `bots/<slug>` (a note in it or the folder itself).
fn mentions(line: &str, slug: &str) -> bool {
    let needle = format!("bots/{slug}");
    line.match_indices(&needle).any(|(i, _)| matches!(line[i + needle.len()..].chars().next(), None | Some('/' | ']' | ')' | '|' | '#' | ' ')))
}

/// INDEX.md without the lines that link to `bots/<slug>`, and without a heading left with nothing under it.
pub fn drop_index_lines(text: &str, slug: &str) -> String {
    let kept: Vec<&str> = text.lines().filter(|l| !mentions(l, slug)).collect();
    if kept.len() == text.lines().count() {
        return text.to_string();
    }
    let mut out: Vec<&str> = vec![];
    for (i, line) in kept.iter().enumerate() {
        // A heading whose section (up to the next heading) the move emptied.
        if line.starts_with('#') && kept[i + 1..].iter().take_while(|l| !l.starts_with('#')).all(|l| l.trim().is_empty()) && section_had(text, line, slug) {
            continue;
        }
        out.push(line);
    }
    // Blank lines left in a row by the removed heading.
    let mut joined: Vec<&str> = vec![];
    for l in out {
        if l.trim().is_empty() && joined.last().is_some_and(|p: &&str| p.trim().is_empty()) {
            continue;
        }
        joined.push(l);
    }
    while joined.last().is_some_and(|l| l.trim().is_empty()) {
        joined.pop();
    }
    let mut s = joined.join("\n");
    if text.ends_with('\n') {
        s.push('\n');
    }
    s
}

/// Whether the section under `heading` in the original text had a line that links to the bot (so only sections
/// the move emptied lose their heading).
fn section_had(text: &str, heading: &str, slug: &str) -> bool {
    let lines: Vec<&str> = text.lines().collect();
    let Some(i) = lines.iter().position(|l| *l == heading) else { return false };
    lines[i + 1..].iter().take_while(|l| !l.starts_with('#')).any(|l| mentions(l, slug))
}

/// The line of the knowledge base's seeded AGENTS.md about bots, before and since bots have their own memory.
const OLD_AGENTS_LINE: &str = "- `bots/<bot>/`: a bot's own notes, short and current (preferences and facts it needs next time, never a journal of its runs).";
pub(crate) const AGENTS_LINE: &str = "- Bots keep their own notes in their own folder (`$LAUNCHPAD_BOT/memory/`, written with `launchpad bot memory`), not here. Facts other agents need go here.";

/// Home, at start, once per bot: moves its notes from the knowledge base's `bots/<slug>/` into `memory/` of its
/// store, removes them from the knowledge base and their lines from INDEX.md (all as Launchpad). Folders of
/// `bots/` that are no bot's stay. Also updates the seeded AGENTS.md line about bots once per workspace. A failure
/// is logged and tried again at the next start.
pub async fn move_from_kb(app: &App) {
    if !crate::home_git::here(app) {
        return;
    }
    for b in crate::bots::list(app).await.unwrap_or_default() {
        let key = format!("bot-memory-from-kb:{}", b.id);
        match crate::ask_bot::done(app, key.clone()).await {
            Ok(false) => {}
            _ => continue,
        }
        match move_bot(app, &b).await {
            Ok(n) => {
                if n > 0 {
                    tracing::info!("moved {n} notes of {} from the knowledge base to its memory", b.settings.name);
                }
                let _ = crate::ask_bot::claim(app, key).await;
            }
            Err(e) => tracing::warn!("moving {}'s notes from the knowledge base: {e:#}", b.settings.name),
        }
    }
    for w in crate::workspace::list(app).await.unwrap_or_default() {
        let key = format!("kb-agents-bots-line:{}", w.id);
        if !matches!(crate::ask_bot::done(app, key.clone()).await, Ok(false)) {
            continue;
        }
        match agents_line(app, &w.id).await {
            Ok(()) => {
                let _ = crate::ask_bot::claim(app, key).await;
            }
            Err(e) => tracing::warn!("AGENTS.md of the knowledge base of {}: {e:#}", w.name),
        }
    }
}

async fn move_bot(app: &App, b: &crate::bots::Bot) -> Result<usize> {
    let prefix = format!("bots/{}/", b.slug);
    let tree = plain::tree(app, &kb::store(&b.workspace_id)).await?;
    let notes: Vec<&plain::Entry> = tree.files.iter().filter(|e| e.path.starts_with(&prefix)).collect();
    if notes.is_empty() {
        return Ok(0);
    }
    let own = plain::tree(app, &crate::bots::store(&b.id)).await?;
    let mut writes = vec![];
    for e in &notes {
        let path = format!("{DIR}{}", &e.path[prefix.len()..]);
        let base = own.files.iter().find(|f| f.path == path).map(|f| f.sha256.clone());
        // An identical file is unchanged; another one is kept as a second version (base None).
        let base = base.filter(|h| *h == e.sha256);
        writes.push(Op::Write { path, sha256: e.sha256.clone(), size: e.size, base });
    }
    apply(app, &b.id, Author::launchpad(), writes).await?;
    let mut removes: Vec<Op> = notes.iter().map(|e| Op::Remove { path: e.path.clone() }).collect();
    if let Some(index) = tree.files.iter().find(|e| e.path == kb::INDEX)
        && let Some(text) = kb::blob_text(app, &index.sha256).await
    {
        let new = drop_index_lines(&text, &b.slug);
        if new != text {
            let (h, size) = kb::put(app, new.as_bytes()).await?;
            removes.push(Op::Write { path: kb::INDEX.into(), sha256: h, size, base: Some(index.sha256.clone()) });
        }
    }
    kb::apply(app, &b.workspace_id, Author::launchpad(), removes).await?;
    Ok(notes.len())
}

async fn agents_line(app: &App, workspace_id: &str) -> Result<()> {
    let tree = plain::tree(app, &kb::store(workspace_id)).await?;
    let Some(e) = tree.files.iter().find(|e| e.path == "AGENTS.md") else { return Ok(()) };
    let Some(text) = kb::blob_text(app, &e.sha256).await else { return Ok(()) };
    if !text.contains(OLD_AGENTS_LINE) {
        return Ok(());
    }
    let (h, size) = kb::put(app, text.replace(OLD_AGENTS_LINE, AGENTS_LINE).as_bytes()).await?;
    kb::apply(app, workspace_id, Author::launchpad(), vec![Op::Write { path: "AGENTS.md".into(), sha256: h, size, base: Some(e.sha256.clone()) }]).await?;
    Ok(())
}

// ---------------------------------------------------------------- runner side

#[derive(Default, Serialize, Deserialize)]
struct Synced {
    files: BTreeMap<String, String>,
}

/// Where this machine keeps a bot's read-only copy (`$LAUNCHPAD_BOT`).
pub fn mirror_dir(app: &App, bot_id: &str) -> PathBuf {
    app.paths.data_dir.join("bot-files").join(bot_id)
}

fn state_file(app: &App, bot_id: &str) -> PathBuf {
    app.paths.data_dir.join("bot-files-state").join(format!("{bot_id}.json"))
}

/// Runner: brings the copy of a bot's store to Home's version (Home wins: the copy is read-only). Returns it.
pub async fn sync(app: &App, bot_id: &str) -> Result<PathBuf> {
    static SYNCS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>> = std::sync::LazyLock::new(Default::default);
    let gate = SYNCS.lock().unwrap().entry(bot_id.to_string()).or_default().clone();
    let _one = gate.lock().await;
    let id = crate::bots::store(bot_id);
    let dir = mirror_dir(app, bot_id);
    tokio::fs::create_dir_all(&dir).await?;
    let state = state_file(app, bot_id);
    let synced: Synced = tokio::fs::read(&state).await.ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let tree = plain::home::tree(app, &id).await?;
    let wanted: BTreeMap<&str, &str> = tree.files.iter().map(|e| (e.path.as_str(), e.sha256.as_str())).collect();
    for rel in plain::walk(&dir) {
        if !wanted.contains_key(rel.as_str()) {
            let _ = tokio::fs::remove_file(dir.join(&rel)).await;
        }
    }
    for e in &tree.files {
        plain::check_path(&e.path)?;
        let path = dir.join(&e.path);
        if synced.files.get(&e.path) == Some(&e.sha256) && path.is_file() {
            continue;
        }
        let data = plain::home::blob(app, &id, &e.sha256).await?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let tmp = path.with_extension("lp-partial");
        tokio::fs::write(&tmp, data).await?;
        tokio::fs::rename(&tmp, &path).await?;
    }
    kb::remove_empty_dirs(&dir).await;
    let synced = Synced { files: tree.files.iter().map(|e| (e.path.clone(), e.sha256.clone())).collect() };
    tokio::fs::create_dir_all(state.parent().unwrap()).await?;
    tokio::fs::write(&state, serde_json::to_vec(&synced)?).await?;
    Ok(dir)
}

/// Runner: keeps the copies of the bots whose sessions are open on this machine current (every minute, with the
/// knowledge bases).
pub async fn sync_open(app: &App) {
    let mut ids: Vec<String> = app.sessions.list().into_iter().filter(|s| s.status != crate::model::SessionStatus::Closed).filter_map(|s| s.launch_spec.bot.map(|b| b.id)).collect();
    ids.sort();
    ids.dedup();
    for id in ids {
        if let Err(e) = sync(app, &id).await {
            tracing::debug!("bot files {id}: {e:#}");
        }
    }
}

/// `launchpad bot memory write|append|rm` from a bot session on this machine: uploads contents, sends the writes
/// to Home, then refreshes the copy.
pub async fn session_write(app: &App, session_id: &str, ops: Vec<LocalOp>) -> Result<Vec<String>> {
    let s = app.sessions.get(session_id).context("no such session")?;
    let bot = s.launch_spec.bot.as_ref().context("this session is not a bot's: it has no bot memory")?;
    let id = crate::bots::store(&bot.id);
    let state: Synced = tokio::fs::read(state_file(app, &bot.id)).await.ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let mut sent = vec![];
    for op in ops {
        sent.push(match op {
            LocalOp::Write { path, content_b64 } => {
                let path = store_path(&path)?;
                let data = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, content_b64)?;
                let h = kb::sha(&data);
                let size = data.len() as u64;
                plain::home::upload(app, &id, &h, data).await?;
                Op::Write { base: state.files.get(&path).cloned(), path, sha256: h, size }
            }
            LocalOp::Append { path, text } => Op::Append { path: store_path(&path)?, text },
            LocalOp::Remove { path } => Op::Remove { path: store_path(&path)? },
        });
    }
    let v = crate::home_link::home_call("bot-memory", json!({"session_id": session_id, "ops": sent})).await?;
    let lines: Vec<String> = serde_json::from_value(v["lines"].clone()).unwrap_or_default();
    if let Err(e) = sync(app, &bot.id).await {
        tracing::warn!("bot files {}: {e:#}", bot.id);
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_under_memory() {
        assert_eq!(store_path("notes.md").unwrap(), "memory/notes.md");
        assert_eq!(store_path("./memory/people/ana.md").unwrap(), "memory/people/ana.md");
        assert!(store_path("../BOT.md").is_err());
        assert!(store_path("").is_err());
        assert!(store_path("memory/").is_err());
    }

    #[test]
    fn index_loses_the_bots_lines_and_emptied_headings() {
        let text = "# Index\n\nThe map.\n\n## How-tos\n- [[howto/x]]: x\n\n## Bots\n- [[bots/scout/notes]]: Scout\n- [[bots/ask-launchpad/notes]]: Ask\n\n## People\n- [[people/ana]]\n";
        let one = drop_index_lines(text, "scout");
        assert_eq!(one, "# Index\n\nThe map.\n\n## How-tos\n- [[howto/x]]: x\n\n## Bots\n- [[bots/ask-launchpad/notes]]: Ask\n\n## People\n- [[people/ana]]\n");
        let both = drop_index_lines(&one, "ask-launchpad");
        assert_eq!(both, "# Index\n\nThe map.\n\n## How-tos\n- [[howto/x]]: x\n\n## People\n- [[people/ana]]\n");
        // Another bot whose slug starts the same, and a section that was already empty, stay.
        let other = "## Bots\n- [[bots/scout-2/notes]]\n\n## Empty\n";
        assert_eq!(drop_index_lines(other, "scout"), other);
        assert_eq!(drop_index_lines("- see bots/scout/ for more\n- [x](bots/scout)\nend\n", "scout"), "end\n");
    }
}

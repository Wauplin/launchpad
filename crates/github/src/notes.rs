//! Private notes about a GitHub item (`launchpad note`): a private timeline of an issue or pull request, layered by the
//! extension into GitHub's own timeline. Never posted anywhere: Home keeps them in state.db (`item_note`). Each write
//! appends a note (what changed since the last one: new commits, comments, the owner's note); several agents and the
//! owner (`note.write`, `by_owner`) write on the same timeline. A note may carry a suggested comment (a record of the
//! Suggestions, posted only by the owner's click) and answer another note (`reply_to`).
use anyhow::{Context, Result, ensure};
use launchpad_core::App;
use serde::{Deserialize, Serialize};
use serde_json::json;
use ts_rs::TS;

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ItemNote {
    pub id: String,
    pub repo: String,
    #[ts(type = "number")]
    pub number: u64,
    pub workspace_id: Option<String>,
    /// The session that wrote it (None: the owner's).
    pub session_id: Option<String>,
    pub bot_id: Option<String>,
    /// The bot's name, else the session's title ("You" for the owner's).
    pub author: String,
    /// The owner's own private note (`note.write`).
    #[serde(default)]
    pub by_owner: bool,
    /// Markdown.
    pub body: String,
    #[ts(type = "number")]
    pub created_at: i64,
    /// The suggested comment it carries (`--suggest-file`).
    #[serde(default)]
    pub suggestion_id: Option<String>,
    /// The note it answers (`--reply-to-note`).
    #[serde(default)]
    pub reply_to: Option<String>,
}

#[derive(Deserialize)]
pub struct Write {
    pub repo: String,
    pub number: u64,
    pub body: String,
    pub session_id: String,
    /// The text of a suggested comment it carries (`--suggest-file`).
    #[serde(default)]
    pub suggest: Option<String>,
    /// That comment answers one comment of the item (`--reply-to`, as `launchpad suggest comment`).
    #[serde(default)]
    pub suggest_reply_to: Option<String>,
    /// The note it answers (`--reply-to-note`).
    #[serde(default)]
    pub reply_to: Option<String>,
}

/// The owner's private note (`note.write`).
#[derive(Deserialize)]
pub struct OwnerWrite {
    pub repo: String,
    pub number: u64,
    pub workspace: String,
    pub body: String,
    #[serde(default)]
    pub reply_to: Option<String>,
}

/// The key of an item in the table: `owner/repo#number`, lowercase.
fn item(repo: &str, number: u64) -> String {
    format!("{}#{number}", repo.to_lowercase())
}

/// The notes of one item, oldest first (old rows of another shape are left out).
pub async fn of_item(app: &App, repo: &str, number: u64) -> Result<Vec<ItemNote>> {
    let key = item(repo, number);
    let mut out: Vec<ItemNote> = app
        .state_db
        .call(move |c| {
            let mut q = c.prepare("SELECT json FROM item_note WHERE item = ?1")?;
            let rows = q.query_map([key], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows.iter().filter_map(|j| serde_json::from_str(j).ok()).collect())
        })
        .await?;
    out.sort_by(|a, b| a.created_at.cmp(&b.created_at).then_with(|| a.id.cmp(&b.id)));
    Ok(out)
}

fn check_body(body: &str) -> Result<()> {
    ensure!(!body.trim().is_empty() && body.len() <= 65_536, "a note holds 1–65536 bytes of text");
    Ok(())
}

/// A note it answers must be about the same item, in the same workspace.
async fn check_reply(app: &App, repo: &str, number: u64, workspace: Option<&str>, reply_to: Option<&str>) -> Result<Option<String>> {
    let Some(id) = reply_to.map(str::trim).filter(|r| !r.is_empty()) else { return Ok(None) };
    ensure!(of_item(app, repo, number).await?.iter().any(|n| n.id == id && n.workspace_id.as_deref() == workspace), "--reply-to-note: no note {id} about {repo}#{number}");
    Ok(Some(id.to_string()))
}

async fn save(app: &App, note: &ItemNote) -> Result<()> {
    let (id, key, text) = (note.id.clone(), item(&note.repo, note.number), serde_json::to_string(note)?);
    app.state_db
        .call(move |c| {
            c.execute("INSERT INTO item_note(id,item,json) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET json=excluded.json", rusqlite::params![id, key, text])?;
            Ok(())
        })
        .await?;
    app.bus.publish("notes", "changed", json!({"repo": note.repo, "number": note.number}));
    Ok(())
}

/// Appends the session's note about `repo#number`, with the suggested comment it carries (created like `launchpad
/// suggest comment`). Links the session to the item like a suggestion does.
pub async fn write(app: &App, q: Write) -> Result<ItemNote> {
    crate::items::validate(&q.repo)?;
    ensure!(q.number > 0, "invalid item number");
    check_body(&q.body)?;
    let s = app.sessions.get(&q.session_id).context("session not found")?;
    let workspace_id = launchpad_core::kb::workspace_of_session(app, &q.session_id).await;
    let reply_to = check_reply(app, &q.repo, q.number, workspace_id.as_deref(), q.reply_to.as_deref()).await?;
    let bot = s.launch_spec.bot.as_ref();
    let suggestion_id = match q.suggest.filter(|t| !t.trim().is_empty()) {
        Some(body) => {
            let create = crate::suggestions::Create {
                target: crate::suggestions::SuggestionTarget::GitHub { repo: q.repo.clone(), number: q.number, reply_to: None },
                reply_to: q.suggest_reply_to,
                kind: crate::suggestions::SuggestionKind::Comment,
                body,
                comments: vec![],
                event: "COMMENT".into(),
                head: None,
                session_id: Some(q.session_id.clone()),
                merge: None,
                run_ci: false,
            };
            Some(crate::suggestions::create(app, create).await?.id)
        }
        None => None,
    };
    let note = ItemNote {
        id: uuid::Uuid::new_v4().to_string(),
        repo: q.repo.clone(),
        number: q.number,
        workspace_id,
        session_id: Some(q.session_id.clone()),
        bot_id: bot.map(|b| b.id.clone()),
        author: bot.map(|b| b.name.clone()).unwrap_or_else(|| s.title.clone()),
        by_owner: false,
        body: q.body,
        created_at: launchpad_core::util::now_ms(),
        suggestion_id,
        reply_to,
    };
    save(app, &note).await?;
    if let Err(e) = launchpad_core::ref_use::link(app, &q.session_id, &q.repo, q.number, None).await {
        tracing::warn!("Note item link: {e:#}");
    }
    Ok(note)
}

/// Stores the owner's private note (`note.write`); the caller tells the agents (`recipients`).
pub async fn write_owner(app: &App, q: &OwnerWrite) -> Result<ItemNote> {
    crate::items::validate(&q.repo)?;
    ensure!(q.number > 0, "invalid item number");
    check_body(&q.body)?;
    ensure!(launchpad_core::workspace::get(app, &q.workspace).await.is_ok(), "no such workspace");
    let reply_to = check_reply(app, &q.repo, q.number, Some(&q.workspace), q.reply_to.as_deref()).await?;
    let note = ItemNote {
        id: uuid::Uuid::new_v4().to_string(),
        repo: q.repo.clone(),
        number: q.number,
        workspace_id: Some(q.workspace.clone()),
        session_id: None,
        bot_id: None,
        author: "You".into(),
        by_owner: true,
        body: q.body.clone(),
        created_at: launchpad_core::util::now_ms(),
        suggestion_id: None,
        reply_to,
    };
    save(app, &note).await?;
    Ok(note)
}

/// The owner deletes a note.
pub async fn delete(app: &App, id: &str) -> Result<()> {
    let key = id.to_string();
    let gone = app.state_db.call(move |c| Ok(c.execute("DELETE FROM item_note WHERE id=?1", [key])?)).await?;
    ensure!(gone > 0, "note not found");
    app.bus.publish("notes", "changed", json!({"id": id}));
    Ok(())
}

/// An agent the owner's note goes to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(rename = "NoteRecipient")]
pub enum Recipient {
    /// A bot: its thread of the item.
    Bot { id: String, name: String },
    /// A session without a bot: its own conversation.
    Session { id: String, name: String },
}

/// Pure: the distinct authors of an item's entries (bot, else session, with a name), in order of first appearance:
/// a bot counts once whichever of its sessions wrote.
pub fn distinct_authors(entries: &[(Option<String>, Option<String>, String)]) -> Vec<Recipient> {
    let mut out: Vec<Recipient> = vec![];
    for (bot, session, name) in entries {
        let r = match (bot, session) {
            (Some(b), _) => Recipient::Bot { id: b.clone(), name: name.clone() },
            (None, Some(s)) => Recipient::Session { id: s.clone(), name: name.clone() },
            (None, None) => continue,
        };
        let seen = out.iter().any(|o| match (o, &r) {
            (Recipient::Bot { id: a, .. }, Recipient::Bot { id: b, .. }) | (Recipient::Session { id: a, .. }, Recipient::Session { id: b, .. }) => a == b,
            _ => false,
        });
        if !seen {
            out.push(r);
        }
    }
    out
}

/// Who hears of the owner's note on `repo#number` in `workspace`: every agent that wrote a note or a suggestion about
/// the item there (its bot, else the session), else the item's default bot (`ext::default_bot`).
pub async fn recipients(app: &App, repo: &str, number: u64, workspace: &str) -> Result<Vec<Recipient>> {
    let mut entries: Vec<(i64, Option<String>, Option<String>, String)> = vec![];
    for n in of_item(app, repo, number).await?.into_iter().filter(|n| !n.by_owner && n.workspace_id.as_deref() == Some(workspace)) {
        entries.push((n.created_at, n.bot_id, n.session_id, n.author));
    }
    for s in crate::suggestions::list(app).await? {
        let crate::suggestions::SuggestionTarget::GitHub { repo: r, number: n, .. } = &s.target else { continue };
        let Some(sid) = s.session_id.as_deref() else { continue };
        if !r.eq_ignore_ascii_case(repo) || *n != number || launchpad_core::kb::workspace_of_session(app, sid).await.as_deref() != Some(workspace) {
            continue;
        }
        let Some(session) = app.sessions.get(sid) else { continue };
        let bot = session.launch_spec.bot.as_ref();
        entries.push((s.created_at, bot.map(|b| b.id.clone()), Some(sid.to_string()), bot.map(|b| b.name.clone()).unwrap_or_else(|| session.title.clone())));
    }
    entries.sort_by_key(|e| e.0);
    let flat: Vec<_> = entries.into_iter().map(|(_, b, s, n)| (b, s, n)).collect();
    // A bot that left the workspace (deleted) hears nothing.
    let bots = launchpad_core::bots::briefs(app, workspace).await.unwrap_or_default();
    // An archived session cannot take a message: the default bot hears it when no one else can.
    let live = |id: &str| app.sessions.get(id).is_some_and(|x| x.status != launchpad_core::model::SessionStatus::Closed);
    let mut out: Vec<Recipient> = distinct_authors(&flat).into_iter().filter(|r| match r { Recipient::Bot { id, .. } => bots.iter().any(|b| &b.id == id), Recipient::Session { id, .. } => live(id) }).collect();
    if out.is_empty() {
        let project = launchpad_core::projects::list(app).await.unwrap_or_default().into_iter().find(|p| p.workspace_id.as_deref() == Some(workspace) && p.gh_repo.as_deref().is_some_and(|r| r.eq_ignore_ascii_case(repo)));
        if let Some(b) = crate::ext::default_bot(app, project.as_ref(), Some(repo), Some(workspace)).await {
            out.push(Recipient::Bot { id: b.id, name: b.name });
        }
    }
    Ok(out)
}

/// The message the owner's note sends each agent: the item, the note quoted, and how to answer.
pub fn owner_message(app: &App, note: &ItemNote, url: &str) -> String {
    let quoted = note.body.lines().map(|l| format!("> {l}")).collect::<Vec<_>>().join("\n");
    format!(
        "The owner added a private note on {repo}#{n} ({url}) in Launchpad (only you agents and the owner see it; never posted on GitHub):\n\n{quoted}\n\nAnswer only if you have something useful to add: read the item again if needed, then write a new private note with `{cli} note --repo {repo} --number {n} --reply-to-note {id} --body-file <file>` (add `--suggest-file <file>` with a comment the owner could post on GitHub). Otherwise end your turn with no_update.",
        repo = note.repo,
        n = note.number,
        id = note.id,
        cli = app.paths.cli_command(),
    )
}

/// Included in every launched session, next to the suggestions' instruction.
pub fn instruction(app: &App, session: &str) -> String {
    format!(
        "Private notes are a private timeline of an issue or pull request, shown to the owner inside GitHub's timeline by the extension and never posted. Run `{cli} note --repo <owner/repo> --number <n> --body-file <file> --session-id {session}` (Markdown) to add a note when something new happened (new commits, new comments, the owner's note); each run appends a new note, so keep each one focused on what changed and never repeat earlier notes. Text the owner could post on GitHub goes with it: add `--suggest-file <file>` (a suggested comment, posted only by the owner's click; `--reply-to <comment>` as for `suggest comment`). Answer the owner's note with `--reply-to-note <note id>`.",
        cli = app.paths.cli_command()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authors_count_once_per_bot_then_per_session() {
        let s = |x: &str| Some(x.to_string());
        let entries = vec![
            (s("b1"), s("s1"), "Reviewer".to_string()),
            (None, s("s2"), "Fix the bug".to_string()),
            (s("b1"), s("s3"), "Reviewer".to_string()),
            (None, s("s2"), "Fix the bug".to_string()),
            (None, None, "nobody".to_string()),
        ];
        assert_eq!(distinct_authors(&entries), vec![Recipient::Bot { id: "b1".into(), name: "Reviewer".into() }, Recipient::Session { id: "s2".into(), name: "Fix the bug".into() }]);
    }
}

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
    /// When the owner saw it (`note.read`: its item's page shown, or Mark read in the extension's panel); None:
    /// unread. The owner's own notes are seen when written.
    #[serde(default)]
    #[ts(type = "number | null")]
    pub seen_at: Option<i64>,
}

impl ItemNote {
    /// An agent's note the owner has not seen yet.
    pub fn unread(&self) -> bool {
        !self.by_owner && self.seen_at.is_none()
    }
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
    /// That comment also closes the item when the owner posts it (`--close`: Comment and close).
    #[serde(default)]
    pub suggest_close: bool,
    /// The actions the owner's Post runs (`--close`, `--reopen`, `--request-reviewers`…): with `suggest`, after that
    /// comment; without it, an action-only suggestion (a close without a comment…).
    #[serde(default)]
    pub suggest_actions: Vec<crate::suggestions::ItemAction>,
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

/// Pure: a note's link, the item's page with the fragment `#lp-note-<id>` (GitHub sends `/issues/N` of a pull request
/// to `/pull/N`, keeping the fragment). Notes reference other notes with it; the extension shows it as a link that
/// scrolls to the note, on its item's page.
pub fn link(repo: &str, number: u64, id: &str) -> String {
    format!("https://github.com/{repo}/issues/{number}#lp-note-{id}")
}

/// A note as agents read it: with its `url` (`link`).
pub fn with_link(note: &ItemNote) -> serde_json::Value {
    let mut v = json!(note);
    v["url"] = json!(link(&note.repo, note.number, &note.id));
    v
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
    let body = q.suggest.filter(|t| !t.trim().is_empty());
    let has_actions = q.suggest_close || !q.suggest_actions.is_empty();
    ensure!(body.is_some() || q.suggest_reply_to.is_none(), "--reply-to needs --suggest-file: the reply the owner posts");
    let suggestion_id = if body.is_some() || has_actions {
        // Actions without a comment make an action-only suggestion (a close without a comment…).
        let kind = if body.is_some() { crate::suggestions::SuggestionKind::Comment } else { crate::suggestions::SuggestionKind::Action };
        let create = crate::suggestions::Create {
            target: crate::suggestions::SuggestionTarget::GitHub { repo: q.repo.clone(), number: q.number, reply_to: None },
            reply_to: q.suggest_reply_to,
            kind,
            body: body.unwrap_or_default(),
            comments: vec![],
            event: "COMMENT".into(),
            head: None,
            session_id: Some(q.session_id.clone()),
            merge: None,
            run_ci: false,
            close: q.suggest_close,
            actions: q.suggest_actions,
        };
        Some(crate::suggestions::create(app, create).await?.id)
    } else {
        None
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
        seen_at: None,
    };
    save(app, &note).await?;
    if let Err(e) = launchpad_core::ref_use::link(app, &q.session_id, &q.repo, q.number, None).await {
        tracing::warn!("Note item link: {e:#}");
    }
    Ok(note)
}

/// A note Home itself writes on the session's behalf about what it did on GitHub at the session's request (`launchpad
/// ci approve`), so the owner sees it in the item's timeline. It carries no suggestion and links the session to the item.
pub async fn record(app: &App, session_id: &str, repo: &str, number: u64, body: String) -> Result<ItemNote> {
    write(app, Write { repo: repo.to_string(), number, body, session_id: session_id.to_string(), suggest: None, suggest_reply_to: None, suggest_close: false, suggest_actions: vec![], reply_to: None }).await
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
        seen_at: Some(launchpad_core::util::now_ms()),
    };
    save(app, &note).await?;
    Ok(note)
}

/// Every note of every item (old rows of another shape are left out). The table stays small: one row per note.
pub async fn all(app: &App) -> Result<Vec<ItemNote>> {
    app.state_db
        .call(|c| {
            let mut q = c.prepare("SELECT json FROM item_note")?;
            let rows = q.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows.iter().filter_map(|j| serde_json::from_str(j).ok()).collect())
        })
        .await
}

/// What `note.read` marks: the notes of one item, or every note of the workspace.
#[derive(Deserialize)]
pub struct Read {
    pub workspace: String,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub number: Option<u64>,
}

/// `note.read`: the owner saw the agents' notes of an item (its page showed them), or Mark all read. Returns how many
/// notes it marked; nothing changes (and no event) when none was unread.
pub async fn mark_read(app: &App, q: &Read) -> Result<usize> {
    let notes = match (&q.repo, q.number) {
        (Some(repo), Some(n)) => {
            crate::items::validate(repo)?;
            of_item(app, repo, n).await?
        }
        (None, None) => all(app).await?,
        _ => anyhow::bail!("give both repo and number, or neither"),
    };
    let now = launchpad_core::util::now_ms();
    let marked: Vec<(String, String)> = notes
        .into_iter()
        .filter(|n| n.unread() && n.workspace_id.as_deref() == Some(q.workspace.as_str()))
        .filter_map(|mut n| {
            n.seen_at = Some(now);
            serde_json::to_string(&n).ok().map(|j| (n.id, j))
        })
        .collect();
    let count = marked.len();
    if count == 0 {
        return Ok(0);
    }
    app.state_db
        .call(move |c| {
            let tx = c.transaction()?;
            for (id, json) in &marked {
                tx.execute("UPDATE item_note SET json=?2 WHERE id=?1", rusqlite::params![id, json])?;
            }
            tx.commit()?;
            Ok(())
        })
        .await?;
    app.bus.publish("notes", "changed", json!({"repo": q.repo, "number": q.number, "read": count}));
    Ok(count)
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
        "The owner added a private note on {repo}#{n} ({url}) in Launchpad (only you agents and the owner see it; never posted on GitHub; its link: {link}):\n\n{quoted}\n\nAnswer only if you have something useful to add: read the item again if needed, then write a new private note with `{cli} note --repo {repo} --number {n} --reply-to-note {id} --body-file <file>` (a comment the owner could post on GitHub goes in `--suggest-file <file>`, never in the note body). Otherwise end your turn with no_update.",
        repo = note.repo,
        n = note.number,
        id = note.id,
        link = link(&note.repo, note.number, &note.id),
        cli = app.paths.cli_command(),
    )
}

/// Included in every launched session, next to the suggestions' instruction.
pub fn instruction(app: &App, session: &str) -> String {
    format!(
        "Private notes are a private timeline of an issue or pull request, shown to the owner inside GitHub's timeline by the extension and never posted. Run `{cli} note --repo <owner/repo> --number <n> --body-file <file> --session-id {session}` (Markdown) to add a note when something new happened (new commits, new comments, the owner's note); each run appends a new note, so keep each one focused on what changed and never repeat earlier notes. Every action you recommend must be one click for the owner, never only prose or a quoted draft in the note (an \"Action: close …\" line or a draft inside the note cannot be clicked): text the owner could post on GitHub (a reply, a closing comment) goes in `--suggest-file <file>`, which the extension shows under the note as a comment the owner posts with one click (`--reply-to <comment>` as for `suggest comment`); every recommended action adds its flag, applied by the same click: `--close` (Comment and close; a pull request closes without merging; an issue may add `completed` or `not-planned`), `--duplicate-of <n>`, `--reopen`, and for pull requests `--request-reviewers <login,org/team>`, `--ready-for-review`, `--convert-to-draft` and `--update-branch`; these flags without `--suggest-file` show a one-click action without a comment (a close without comment); a recommended approval, merge, CI run or request for changes is a separate `suggest review` with `--event approve|request-changes`, `--merge` or `--run-ci`, which shows in the same timeline. Answer the owner's note with `--reply-to-note <note id>`. To point at another note (of this item or another one), put its link in your Markdown (`url` of `{cli} notes --repo <owner/repo> --number <n>`, which lists the item's notes; `note` prints the new note's `url`); links to issues, pull requests and comments (`#123`, `owner/repo#123`, GitHub URLs) show like on GitHub.",
        cli = app.paths.cli_command()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_links_name_the_item_and_the_note() {
        assert_eq!(link("o/r", 12, "n1"), "https://github.com/o/r/issues/12#lp-note-n1");
    }

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

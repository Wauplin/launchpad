//! Private notes about a GitHub item (`launchpad note`): what an agent wants the owner to know about an issue or pull
//! request before acting on it (a summary, the risks, what to check). Never posted anywhere: Home keeps them in
//! state.db (`item_note`) and the extension shows them on the item's page; a suggested comment is the text to post.
//! One note per author (the bot, else the session) and item in a workspace: a new note of the same author replaces it.
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
    /// The session that wrote the last version.
    pub session_id: Option<String>,
    pub bot_id: Option<String>,
    /// The bot's name, else the session's title.
    pub author: String,
    /// Markdown.
    pub body: String,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

#[derive(Deserialize)]
pub struct Write {
    pub repo: String,
    pub number: u64,
    pub body: String,
    pub session_id: String,
}

/// The key of an item in the table: `owner/repo#number`, lowercase.
fn item(repo: &str, number: u64) -> String {
    format!("{}#{number}", repo.to_lowercase())
}

/// The notes of one item, newest first.
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
    out.sort_by_key(|n| std::cmp::Reverse(n.updated_at));
    Ok(out)
}

/// Writes the session's note about `repo#number`: replaces the note its bot (else the session) already wrote about it
/// in the same workspace. Links the session to the item like a suggestion does.
pub async fn write(app: &App, q: Write) -> Result<ItemNote> {
    crate::items::validate(&q.repo)?;
    ensure!(q.number > 0, "invalid item number");
    ensure!(!q.body.trim().is_empty() && q.body.len() <= 65_536, "a note holds 1–65536 bytes of text");
    let s = app.sessions.get(&q.session_id).context("session not found")?;
    let workspace_id = launchpad_core::kb::workspace_of_session(app, &q.session_id).await;
    let bot = s.launch_spec.bot.as_ref();
    let bot_id = bot.map(|b| b.id.clone());
    let author = bot.map(|b| b.name.clone()).unwrap_or_else(|| s.title.clone());
    let now = launchpad_core::util::now_ms();
    let earlier = of_item(app, &q.repo, q.number).await?.into_iter().find(|n| {
        n.workspace_id == workspace_id
            && match &bot_id {
                Some(b) => n.bot_id.as_ref() == Some(b),
                None => n.bot_id.is_none() && n.session_id.as_deref() == Some(q.session_id.as_str()),
            }
    });
    let note = ItemNote {
        id: earlier.as_ref().map(|n| n.id.clone()).unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        repo: q.repo.clone(),
        number: q.number,
        workspace_id,
        session_id: Some(q.session_id.clone()),
        bot_id,
        author,
        body: q.body,
        created_at: earlier.map(|n| n.created_at).unwrap_or(now),
        updated_at: now,
    };
    let (id, key, text) = (note.id.clone(), item(&note.repo, note.number), serde_json::to_string(&note)?);
    app.state_db
        .call(move |c| {
            c.execute("INSERT INTO item_note(id,item,json) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET json=excluded.json", rusqlite::params![id, key, text])?;
            Ok(())
        })
        .await?;
    if let Err(e) = launchpad_core::ref_use::link(app, &q.session_id, &q.repo, q.number, None).await {
        tracing::warn!("Note item link: {e:#}");
    }
    app.bus.publish("notes", "changed", json!({"repo": note.repo, "number": note.number}));
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

/// Included in every launched session, next to the suggestions' instruction.
pub fn instruction(app: &App, session: &str) -> String {
    format!(
        "To leave the owner a private note about an issue or pull request (never posted: a summary, what matters, risks, what to check or decide), run `{} note --repo <owner/repo> --number <n> --body-file <file> --session-id {session}` (Markdown; it replaces your earlier note about that item). The extension shows it on the item's page under its description. Text meant for GitHub goes into a suggested comment instead.",
        app.paths.cli_command()
    )
}

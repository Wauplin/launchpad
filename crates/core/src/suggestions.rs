//! Agent text stays local until an explicit owner publication action.
use crate::{App, items, predrafts};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::json;
use ts_rs::TS;

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SuggestionTarget {
    GitHub { repo: String, number: u64 },
    Predraft { id: String },
}
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionKind {
    Comment,
    Review,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Suggestion {
    pub id: String,
    pub target: SuggestionTarget,
    pub kind: SuggestionKind,
    pub body: String,
    pub event: String,
    pub head: Option<String>,
    pub author: String,
    pub session_id: Option<String>,
    pub created_at: i64,
    /// A durable claim prevents duplicate submissions after a crash or lost response.
    pub status: String,
    pub url: Option<String>,
}
#[derive(Deserialize)]
pub struct Create {
    pub target: SuggestionTarget,
    pub kind: SuggestionKind,
    pub body: String,
    #[serde(default = "comment_event")]
    pub event: String,
    pub head: Option<String>,
    pub session_id: Option<String>,
}
fn comment_event() -> String {
    "COMMENT".into()
}
#[derive(Deserialize)]
pub struct Edit {
    pub body: String,
    pub event: String,
}
fn validate(body: &str, event: &str, kind: &SuggestionKind) -> Result<()> {
    ensure!(
        !body.trim().is_empty() && body.len() <= 65_536,
        "suggestion must contain 1–65536 bytes of text"
    );
    ensure!(
        ["COMMENT", "APPROVE", "REQUEST_CHANGES"].contains(&event),
        "invalid review event"
    );
    ensure!(
        *kind == SuggestionKind::Review || event == "COMMENT",
        "comments cannot approve or request changes"
    );
    Ok(())
}
pub async fn list(app: &App) -> Result<Vec<Suggestion>> {
    app.state_db
        .call(|c| {
            let mut st = c.prepare("SELECT json FROM draft_comment ORDER BY rowid")?;
            let rows = st
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            rows.into_iter()
                .map(|s| Ok(serde_json::from_str(&s)?))
                .collect()
        })
        .await
}
async fn get(app: &App, id: &str) -> Result<Suggestion> {
    list(app)
        .await?
        .into_iter()
        .find(|s| s.id == id)
        .context("suggestion not found")
}
async fn save(app: &App, s: &Suggestion) -> Result<()> {
    let id = s.id.clone();
    let text = serde_json::to_string(s)?;
    app.state_db
        .call(move |c| {
            c.execute(
                "INSERT INTO draft_comment(id,json) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET json=excluded.json",
                rusqlite::params![id, text],
            )?;
            Ok(())
        })
        .await?;
    app.bus.publish("suggestions", "updated", json!(s));
    Ok(())
}
pub async fn create(app: &App, q: Create) -> Result<Suggestion> {
    validate(&q.body, &q.event, &q.kind)?;
    match &q.target {
        SuggestionTarget::GitHub { repo, number } => {
            items::validate(repo)?;
            ensure!(*number > 0, "invalid item number");
        }
        SuggestionTarget::Predraft { id } => {
            let p = predrafts::get(app, id).await?;
            ensure!(
                !p.archived && p.published_url.is_none(),
                "pre-draft is archived or published"
            );
        }
    }
    // Never guess the reviewed head from a newer cached PR.
    if q.kind == SuggestionKind::Review {
        ensure!(
            q.head
                .as_ref()
                .is_some_and(|h| h.len() == 40 && h.bytes().all(|b| b.is_ascii_hexdigit())),
            "reviews require --head with the inspected commit id"
        );
    }
    let author = match q.session_id.as_deref() {
        Some(id) => match app.sessions.get(id).context("session not found")?.harness {
            crate::model::HarnessKind::Claude => "Claude",
            crate::model::HarnessKind::Codex => "Codex",
        },
        None => "Agent",
    }
    .to_string();
    let s = Suggestion {
        id: uuid::Uuid::new_v4().to_string(),
        target: q.target,
        kind: q.kind,
        body: q.body,
        event: q.event,
        head: q.head,
        author,
        session_id: q.session_id,
        created_at: crate::util::now_ms(),
        status: "draft".into(),
        url: None,
    };
    save(app, &s).await?;
    Ok(s)
}
pub async fn edit(app: &App, id: &str, q: Edit) -> Result<Suggestion> {
    let _guard = app.predraft_lock.lock().await;
    let mut s = get(app, id).await?;
    ensure!(s.status == "draft", "only unposted drafts can be edited");
    validate(&q.body, &q.event, &s.kind)?;
    s.body = q.body;
    s.event = q.event;
    save(app, &s).await?;
    Ok(s)
}
pub async fn discard(app: &App, id: &str) -> Result<()> {
    let _guard = app.predraft_lock.lock().await;
    let s = get(app, id).await?;
    ensure!(s.status != "posted", "suggestion is already posted");
    let id = id.to_string();
    app.state_db
        .call(move |c| {
            c.execute("DELETE FROM draft_comment WHERE id=?1", [id])?;
            Ok(())
        })
        .await?;
    app.bus
        .publish("suggestions", "discarded", json!({"id":s.id}));
    Ok(())
}
pub async fn post(app: &App, id: &str) -> Result<Suggestion> {
    let _guard = app.predraft_lock.lock().await;
    let mut s = get(app, id).await?;
    ensure!(
        s.status == "draft",
        "publication already attempted; check GitHub before discarding or creating another suggestion"
    );
    match &s.target {
        SuggestionTarget::Predraft { id } => {
            let mut p = predrafts::get(app, id).await?;
            ensure!(
                !p.archived && p.published_url.is_none() && p.pushed_head.is_none(),
                "pre-draft is no longer editable"
            );
            p.notes.push(predrafts::Note {
                path: None,
                text: format!(
                    "Suggested by {}{}:\n{}",
                    s.author,
                    s.head
                        .as_ref()
                        .map(|h| format!(" at {h}"))
                        .unwrap_or_default(),
                    s.body
                ),
            });
            s.status = "posted".into();
            let predraft_id = id.clone();
            let predraft_json = serde_json::to_string(&p)?;
            let suggestion_id = s.id.clone();
            let suggestion_json = serde_json::to_string(&s)?;
            app.state_db
                .call(move |c| {
                    let tx = c.transaction()?;
                    tx.execute(
                        "UPDATE predraft SET json=?1 WHERE id=?2",
                        rusqlite::params![predraft_json, predraft_id],
                    )?;
                    tx.execute(
                        "UPDATE draft_comment SET json=?1 WHERE id=?2",
                        rusqlite::params![suggestion_json, suggestion_id],
                    )?;
                    tx.commit()?;
                    Ok(())
                })
                .await?;
            app.bus.publish("predrafts", "updated", json!(p));
            app.bus.publish("suggestions", "updated", json!(s));
            return Ok(s);
        }
        SuggestionTarget::GitHub { repo, number } => {
            if s.kind == SuggestionKind::Review {
                let current = items::detail(app, repo, *number, true).await?;
                ensure!(
                    current["__typename"] == "PullRequest",
                    "reviews require a pull request"
                );
                ensure!(
                    current["headRefOid"].as_str() == s.head.as_deref(),
                    "PR head changed; ask the agent for a fresh review"
                );
                ensure!(current["state"] == "OPEN", "pull request is no longer open");
            }
            s.status = "posting".into();
            save(app, &s).await?;
            let action = if s.kind == SuggestionKind::Review {
                "review"
            } else {
                "comment"
            };
            let result = items::mutate(
                app,
                repo,
                *number,
                action,
                json!({"body":s.body,"event":s.event,"commit_id":s.head}),
            )
            .await;
            match result {
                Ok(v) => s.url = v["html_url"].as_str().map(str::to_string),
                Err(e) => {
                    s.status = "uncertain".into();
                    save(app, &s).await?;
                    return Err(e.context("Posting did not complete locally. Check GitHub before creating another draft; this suggestion will not be retried automatically."));
                }
            }
        }
    }
    s.status = "posted".into();
    save(app, &s).await?;
    Ok(s)
}
/// Included in every launched session, with the exact daemon and session identity.
pub fn instruction(app: &App, session: &str) -> String {
    format!(
        "Agent-written GitHub comments and reviews must stay local. Use the launchpad skill and `{} suggest comment|review --repo <owner/repo> --number <n> --body-file <file> --session-id {session}`. Reviews also require --head <inspected-40-character-commit> and optionally --event approve|request-changes. For local PRs use --predraft <id> instead of --repo/--number. Never publish suggestions yourself; the owner edits and posts with a click.",
        app.paths.cli_command()
    )
}

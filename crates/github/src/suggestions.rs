//! Agent text stays local until an explicit owner publication action.
use crate::items;
use launchpad_core::{App, predrafts};
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
/// A line comment of a suggested review: GitHub's review comment fields. `side` is `RIGHT` (the new file) or `LEFT`
/// (a removed line); `start_line` makes it span lines `start_line..=line` on the same side (for a multi-line
/// ```suggestion block).
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct LineComment {
    pub path: String,
    pub line: u64,
    #[serde(default = "right_side")]
    pub side: String,
    #[serde(default)]
    #[ts(optional)]
    pub start_line: Option<u64>,
    pub body: String,
}
fn right_side() -> String {
    "RIGHT".into()
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Suggestion {
    pub id: String,
    pub target: SuggestionTarget,
    pub kind: SuggestionKind,
    pub body: String,
    /// Line comments of a review, posted with it in one GitHub review (on a Local PR: notes on those lines).
    #[serde(default)]
    pub comments: Vec<LineComment>,
    pub event: String,
    pub head: Option<String>,
    pub author: String,
    pub session_id: Option<String>,
    pub created_at: i64,
    /// A durable claim prevents duplicate submissions after a crash or lost response.
    pub status: String,
    pub url: Option<String>,
    /// A review that approves can also merge the pull request (merge, squash or rebase) at the reviewed head, right
    /// after the review is posted, with the same click.
    #[serde(default)]
    pub merge: Option<String>,
    /// A review can also approve the GitHub Actions runs of the reviewed head that wait for a maintainer.
    #[serde(default)]
    pub run_ci: bool,
    /// What failed after the review was posted (the merge or the workflow approval). Never retried.
    #[serde(default)]
    pub followup_error: Option<String>,
}
#[derive(Deserialize)]
pub struct Create {
    pub target: SuggestionTarget,
    pub kind: SuggestionKind,
    pub body: String,
    #[serde(default)]
    pub comments: Vec<LineComment>,
    #[serde(default = "comment_event")]
    pub event: String,
    pub head: Option<String>,
    pub session_id: Option<String>,
    #[serde(default)]
    pub merge: Option<String>,
    #[serde(default)]
    pub run_ci: bool,
}
fn comment_event() -> String {
    "COMMENT".into()
}
#[derive(Deserialize)]
pub struct Edit {
    pub body: String,
    /// The line comments to keep (the owner removes some); absent keeps them all.
    #[serde(default)]
    pub comments: Option<Vec<LineComment>>,
    pub event: String,
    #[serde(default)]
    pub merge: Option<String>,
    #[serde(default)]
    pub run_ci: bool,
}
/// The follow-ups of a review: merge only after an approval, both only on a GitHub pull request, never both (the
/// workflows would still run when the merge happens).
fn validate_followups(kind: &SuggestionKind, event: &str, target: &SuggestionTarget, merge: Option<&str>, run_ci: bool) -> Result<()> {
    if merge.is_none() && !run_ci {
        return Ok(());
    }
    ensure!(*kind == SuggestionKind::Review && matches!(target, SuggestionTarget::GitHub { .. }), "only reviews of GitHub pull requests can merge or run CI");
    ensure!(!(merge.is_some() && run_ci), "merge or run CI, not both: merge after the checks passed");
    if let Some(m) = merge {
        ensure!(["merge", "squash", "rebase"].contains(&m), "merge method is merge, squash or rebase");
        ensure!(event == "APPROVE", "only an approving review can merge");
    }
    Ok(())
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
/// At most 100 line comments, only on a review, each on a relative path with a valid range and 1–65536 bytes of text.
fn validate_comments(kind: &SuggestionKind, comments: &[LineComment]) -> Result<()> {
    if comments.is_empty() {
        return Ok(());
    }
    ensure!(*kind == SuggestionKind::Review, "only reviews have line comments");
    ensure!(comments.len() <= 100, "at most 100 line comments");
    for c in comments {
        let at = format!("{}:{}", c.path, c.line);
        ensure!(!c.path.is_empty() && !c.path.starts_with('/') && !c.path.split('/').any(|p| p.is_empty() || p == "." || p == ".."), "line comment {at}: the path must be relative to the repository root");
        ensure!(c.line > 0, "line comment {at}: lines start at 1");
        ensure!(c.side == "RIGHT" || c.side == "LEFT", "line comment {at}: side is RIGHT or LEFT");
        ensure!(c.start_line.is_none_or(|s| s > 0 && s < c.line), "line comment {at}: start_line must be before line");
        ensure!(!c.body.trim().is_empty() && c.body.len() <= 65_536, "line comment {at}: 1–65536 bytes of text");
    }
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
    validate_comments(&q.kind, &q.comments)?;
    validate_followups(&q.kind, &q.event, &q.target, q.merge.as_deref(), q.run_ci)?;
    match &q.target {
        SuggestionTarget::GitHub { repo, number } => {
            items::validate(repo)?;
            ensure!(*number > 0, "invalid item number");
        }
        SuggestionTarget::Predraft { id } => {
            let p = predrafts::get(app, id).await?;
            ensure!(
                p.is_open(),
                "this Local PR is archived, published or merged locally"
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
        Some(id) => app
            .sessions
            .get(id)
            .context("session not found")?
            .harness
            .display(),
        None => "Agent",
    }
    .to_string();
    let s = Suggestion {
        id: uuid::Uuid::new_v4().to_string(),
        target: q.target,
        kind: q.kind,
        body: q.body,
        comments: q.comments,
        event: q.event,
        head: q.head.map(|h| h.to_ascii_lowercase()),
        author,
        session_id: q.session_id,
        created_at: launchpad_core::util::now_ms(),
        status: "draft".into(),
        url: None,
        merge: q.merge,
        run_ci: q.run_ci,
        followup_error: None,
    };
    save(app, &s).await?;
    Ok(s)
}
pub async fn edit(app: &App, id: &str, q: Edit) -> Result<Suggestion> {
    let lock = app.suggestion_locks.get(id);
    let _guard = lock.lock().await;
    let mut s = get(app, id).await?;
    ensure!(s.status == "draft", "only unposted drafts can be edited");
    validate(&q.body, &q.event, &s.kind)?;
    validate_followups(&s.kind, &q.event, &s.target, q.merge.as_deref(), q.run_ci)?;
    if let Some(comments) = q.comments {
        // The owner removes line comments; the agent's text and places stay as written.
        ensure!(comments.iter().all(|c| s.comments.contains(c)), "line comments can only be removed");
        s.comments = comments;
    }
    s.body = q.body;
    s.event = q.event;
    s.merge = q.merge;
    s.run_ci = q.run_ci;
    save(app, &s).await?;
    Ok(s)
}
pub async fn discard(app: &App, id: &str) -> Result<()> {
    let lock = app.suggestion_locks.get(id);
    let _guard = lock.lock().await;
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
    let lock = app.suggestion_locks.get(id);
    let _guard = lock.lock().await;
    let mut s = get(app, id).await?;
    ensure!(
        s.status == "draft",
        "publication already attempted; check GitHub before discarding or creating another suggestion"
    );
    match &s.target {
        SuggestionTarget::Predraft { id } => {
            let _predraft = app.predraft_lock.lock().await;
            let mut p = predrafts::get(app, id).await?;
            ensure!(
                p.is_open() && p.pushed_head.is_none(),
                "this Local PR can no longer be edited"
            );
            p.notes.push(predrafts::Note {
                path: None,
                line: None,
                side: None,
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
            for c in &s.comments {
                let lines = c.start_line.map(|from| format!("Lines {from}–{}: ", c.line)).unwrap_or_default();
                p.notes.push(predrafts::Note {
                    path: Some(c.path.clone()),
                    line: Some(c.line),
                    side: Some(if c.side == "LEFT" { "old" } else { "new" }.into()),
                    text: format!("{lines}{}", c.body),
                });
            }
            s.status = "posted".into();
            // On a runner, the Local PR is Home's: the note goes there, then the suggestion is marked here.
            if !launchpad_core::home_git::here(app) {
                let p = predrafts::update_anywhere(app, id, predrafts::Patch { notes: Some(p.notes.clone()), ..Default::default() }).await?;
                let (suggestion_id, suggestion_json) = (s.id.clone(), serde_json::to_string(&s)?);
                app.state_db.call(move |c| Ok(c.execute("UPDATE draft_comment SET json=?1 WHERE id=?2", rusqlite::params![suggestion_json, suggestion_id])?)).await?;
                app.bus.publish("predrafts", "updated", json!(p));
                app.bus.publish("suggestions", "updated", json!(s));
                return Ok(s);
            }
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
            // The agent's attribution line ends what reaches GitHub; the draft keeps the text as written.
            let body = match launchpad_core::attribution::for_session(app, s.session_id.as_deref()) {
                Some(line) => launchpad_core::attribution::sign(&s.body, &line, launchpad_core::attribution::Separator::Rule),
                None => s.body.clone(),
            };
            let result = items::mutate(
                app,
                repo,
                *number,
                action,
                json!({"body":body,"event":s.event,"commit_id":s.head,"comments":s.comments}),
            )
            .await;
            match result {
                Ok(v) => s.url = v["html_url"].as_str().map(str::to_string),
                Err(e) => {
                    let uncertain = launchpad_core::github::delivery_uncertain(&e);
                    s.status = if uncertain { "uncertain" } else { "draft" }.into();
                    save(app, &s).await?;
                    return Err(e.context(if uncertain {
                        "Posting did not complete locally. Check GitHub before creating another draft; this suggestion will not be retried automatically."
                    } else {
                        "GitHub did not accept the post. This suggestion remains editable and can be posted again."
                    }));
                }
            }
        }
    }
    s.status = "posted".into();
    save(app, &s).await?;
    if let SuggestionTarget::GitHub { repo, number } = &s.target {
        // The follow-ups run once, after the review is saved as posted: a failure stays on the card, never retried.
        let head = s.head.clone().unwrap_or_default();
        let followup = if s.run_ci {
            items::mutate(app, repo, *number, "run-ci", json!({"head": head})).await.err().map(|e| format!("Running the workflows failed: {e:#}"))
        } else if let Some(method) = &s.merge {
            items::mutate(app, repo, *number, "merge", json!({"sha": head, "merge_method": method})).await.err().map(|e| format!("The merge failed: {e:#}. Check the pull request on GitHub before merging from its page."))
        } else {
            None
        };
        if let Some(error) = followup {
            s.followup_error = Some(error.clone());
            save(app, &s).await?;
            anyhow::bail!("The review was posted. {error}");
        }
    }
    Ok(s)
}
/// Included in every launched session, with the exact daemon and session identity (followed by the channels'
/// instruction, when the owner has some).
pub fn instruction(app: &App, session: &str) -> String {
    format!(
        "Agent-written GitHub comments and reviews must stay local. Use the launchpad skill and `{} suggest comment|review --repo <owner/repo> --number <n> --body-file <file> --session-id {session}`. Reviews also require --head <inspected-40-character-commit> and optionally --event approve|request-changes, and may add --comments-file <file.json> with inline line comments posted in the same review (a JSON array of {{path, line, side: RIGHT or LEFT for a removed line, start_line (optional, for a range), body}}; a body may hold a ```suggestion block); put each finding about specific lines in a line comment and keep the --body-file for the overall summary; an approving review may add --merge squash|merge|rebase (the owner's Post click then merges at that head), and a review may add --run-ci (Post also approves the head's GitHub Actions runs that wait for a maintainer). For local PRs use --predraft <id> instead of --repo/--number. Never publish suggestions yourself; the owner edits and posts with a click.",
        app.paths.cli_command()
    )
}

/// Posts the owner's combined review. A source suggestion is claimed before publication, so its other entry
/// points cannot post a second review. Follow-ups run once after the review is known to have been posted.
pub async fn submit_review(app: &App, repo: &str, number: u64, mut body: serde_json::Value) -> Result<serde_json::Value> {
    let source = body["source"].as_str().map(str::to_owned);
    let lock = app.suggestion_locks.get(source.as_deref().unwrap_or("owner-review"));
    let _guard = lock.lock().await;
    let mut suggestion = if let Some(id) = &source {
        let s = get(app, id).await?;
        ensure!(s.status == "draft", "publication already attempted; check GitHub");
        ensure!(s.kind == SuggestionKind::Review && s.target == (SuggestionTarget::GitHub { repo: repo.to_string(), number }), "review source belongs to another item");
        ensure!(s.head.as_deref() == body["commit_id"].as_str(), "reviewed commit changed");
        Some(s)
    } else { None };
    let current = items::detail(app, repo, number, true).await?;
    ensure!(current["state"] == "OPEN" && current["headRefOid"] == body["commit_id"], "PR head changed or PR closed; inspect it again");
    let merge = body["merge"].as_bool().unwrap_or(false);
    let run_ci = body["run_ci"].as_bool().unwrap_or(false);
    ensure!(!merge || body["event"] == "APPROVE", "only an approving review can merge");
    ensure!(!(merge && run_ci), "choose merge or run CI");
    if let Some(s) = &mut suggestion {
        s.body = body["body"].as_str().unwrap_or_default().to_owned();
        s.event = body["event"].as_str().unwrap_or_default().to_owned();
        s.merge = merge.then(|| "squash".to_owned());
        s.run_ci = run_ci;
        s.status = "posting".into();
        save(app, s).await?;
        if let Some(line) = launchpad_core::attribution::for_session(app, s.session_id.as_deref()) {
            body["body"] = json!(launchpad_core::attribution::sign(body["body"].as_str().unwrap_or_default(), &line, launchpad_core::attribution::Separator::Rule));
        }
    }
    let result = items::mutate(app, repo, number, "review", body.clone()).await;
    let mut result = match result {
        Ok(v) => v,
        Err(e) => {
            // A refusal posted nothing: the source stays a draft. Only an incomplete delivery blocks it.
            let uncertain = launchpad_core::github::delivery_uncertain(&e);
            if let Some(s) = &mut suggestion {
                s.status = if uncertain { "uncertain" } else { "draft" }.into();
                save(app, s).await?;
            }
            return Err(e.context(if uncertain {
                "Posting did not complete. Check GitHub before another attempt; it will not be retried automatically."
            } else {
                "GitHub did not accept the review. Nothing was posted; your review stays editable."
            }));
        }
    };
    if let Some(s) = &mut suggestion {
        s.status = "posted".into();
        s.url = result["html_url"].as_str().map(str::to_owned);
        save(app, s).await?;
    }
    let followup = if merge {
        items::mutate(app, repo, number, "merge", json!({"sha":body["commit_id"],"merge_method":"squash"})).await.err()
    } else if run_ci {
        items::mutate(app, repo, number, "run-ci", json!({"head":body["commit_id"]})).await.err()
    } else { None };
    if let Some(e) = followup {
        let message = format!("Review posted. Follow-up failed: {e:#}. Check GitHub; it will not be retried.");
        result["followup_error"] = json!(message);
        if let Some(s) = &mut suggestion { s.followup_error = Some(message); save(app, s).await?; }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn followups_need_an_approving_github_review() {
        let gh = SuggestionTarget::GitHub { repo: "o/r".into(), number: 1 };
        let local = SuggestionTarget::Predraft { id: "p".into() };
        let (review, comment) = (SuggestionKind::Review, SuggestionKind::Comment);
        assert!(validate_followups(&comment, "COMMENT", &gh, None, false).is_ok());
        assert!(validate_followups(&review, "APPROVE", &gh, Some("squash"), false).is_ok());
        assert!(validate_followups(&review, "COMMENT", &gh, Some("squash"), false).is_err());
        assert!(validate_followups(&review, "APPROVE", &gh, Some("fast-forward"), false).is_err());
        assert!(validate_followups(&review, "APPROVE", &local, Some("squash"), false).is_err());
        assert!(validate_followups(&review, "COMMENT", &gh, None, true).is_ok());
        assert!(validate_followups(&comment, "COMMENT", &gh, None, true).is_err());
        assert!(validate_followups(&review, "APPROVE", &gh, Some("merge"), true).is_err());
    }

    #[test]
    fn line_comments_need_a_review_and_a_valid_place() {
        let c = |path: &str, line, side: &str, start_line| LineComment { path: path.into(), line, side: side.into(), start_line, body: "Use `?`.".into() };
        let review = SuggestionKind::Review;
        assert!(validate_comments(&review, &[c("src/a.rs", 3, "RIGHT", None), c("b.ts", 9, "LEFT", Some(7))]).is_ok());
        assert!(validate_comments(&SuggestionKind::Comment, &[c("a.rs", 1, "RIGHT", None)]).is_err());
        for bad in [c("/etc/a", 1, "RIGHT", None), c("a/../b", 1, "RIGHT", None), c("a//b", 1, "RIGHT", None), c("a", 0, "RIGHT", None), c("a", 2, "BOTH", None), c("a", 2, "RIGHT", Some(2)), c("a", 2, "RIGHT", Some(0))] {
            assert!(validate_comments(&review, &[bad]).is_err());
        }
        assert!(validate_comments(&review, &[LineComment { body: " ".into(), ..c("a", 1, "RIGHT", None) }]).is_err());
    }
}

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
    GitHub {
        repo: String,
        number: u64,
        /// A comment that answers one comment of the item (`launchpad suggest comment --reply-to`): posted into that
        /// comment's review thread, or as an issue comment that links the comment.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        reply_to: Option<ReplyTo>,
    },
    Predraft { id: String },
}
/// The comment a suggested reply answers.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct ReplyTo {
    pub kind: ReplyKind,
    /// GitHub's comment ID (`#issuecomment-<id>`, `#discussion_r<id>` in its URL).
    #[ts(type = "number")]
    pub id: u64,
    /// The comment's author login, when GitHub could be asked.
    pub author: Option<String>,
    pub url: String,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReplyKind {
    /// A comment of the conversation: the reply is a new comment of the item that links it.
    IssueComment,
    /// A comment of a review thread on the diff: the reply joins that thread.
    ReviewComment,
}
impl ReplyTo {
    /// The first line of an issue-comment reply (a thread reply needs none): it links the comment and mentions its
    /// author. Launchpad's Post and the extension's Insert both start the text with it.
    pub fn prefix(&self) -> String {
        match (self.kind, &self.author) {
            (ReplyKind::ReviewComment, _) => String::new(),
            (ReplyKind::IssueComment, Some(a)) => format!("> In reply to @{a}: {}\n\n", self.url),
            (ReplyKind::IssueComment, None) => format!("> In reply to {}\n\n", self.url),
        }
    }
}
/// What `--reply-to` names: `issuecomment-<id>`, `discussion_r<id>`, a comment URL with one of them, a review thread's
/// node ID (`PRRT_…`) or a bare comment ID (looked up on GitHub).
enum ReplyRef {
    Comment(Option<ReplyKind>, u64),
    Thread(String),
}
fn parse_reply(raw: &str) -> Result<ReplyRef> {
    let s = raw.trim();
    let s = s.rsplit_once('#').map_or(s, |(_, f)| f);
    let id = |d: &str| d.parse::<u64>().ok().filter(|n| *n > 0);
    if let Some(n) = s.strip_prefix("issuecomment-").and_then(id) {
        return Ok(ReplyRef::Comment(Some(ReplyKind::IssueComment), n));
    }
    if let Some(n) = s.strip_prefix("discussion_r").and_then(id) {
        return Ok(ReplyRef::Comment(Some(ReplyKind::ReviewComment), n));
    }
    if let Some(n) = id(s) {
        return Ok(ReplyRef::Comment(None, n));
    }
    if s.starts_with("PRRT_") && s.len() < 100 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
        return Ok(ReplyRef::Thread(s.to_string()));
    }
    anyhow::bail!("--reply-to takes issuecomment-<id>, discussion_r<id>, a comment URL, a review thread ID (PRRT_…) or a comment ID")
}
/// The comment ID at the end of a GitHub comment URL, with its kind.
pub(crate) fn comment_of(url: &str) -> Option<(ReplyKind, u64)> {
    let (_, f) = url.rsplit_once('#')?;
    if let Some(n) = f.strip_prefix("issuecomment-") {
        return n.parse().ok().map(|n| (ReplyKind::IssueComment, n));
    }
    f.strip_prefix("discussion_r").and_then(|n| n.parse().ok()).map(|n| (ReplyKind::ReviewComment, n))
}
/// Comment `id` of `repo` read from GitHub now: its author and URL. `Ok(None)`: GitHub has no such comment of that
/// kind on item `number`.
async fn read_comment(app: &App, repo: &str, number: u64, kind: ReplyKind, id: u64) -> Result<Option<ReplyTo>> {
    let (path, item) = match kind {
        ReplyKind::IssueComment => (format!("/repos/{repo}/issues/comments/{id}"), ("issue_url", format!("/issues/{number}"))),
        ReplyKind::ReviewComment => (format!("/repos/{repo}/pulls/comments/{id}"), ("pull_request_url", format!("/pulls/{number}"))),
    };
    let v = match app.github.get(&path).await {
        Ok(v) => v,
        Err(e) if e.downcast_ref::<reqwest::Error>().and_then(|e| e.status()) == Some(reqwest::StatusCode::NOT_FOUND) => return Ok(None),
        Err(e) => return Err(e),
    };
    // A comment of another item counts as none: a bare ID may name a comment of each kind.
    if !v[item.0].as_str().is_some_and(|u| u.ends_with(&item.1)) {
        return Ok(None);
    }
    Ok(Some(ReplyTo { kind, id, author: v["user"]["login"].as_str().map(str::to_string), url: v["html_url"].as_str().map(str::to_string).unwrap_or_else(|| reply_url(repo, number, kind, id)) }))
}
fn reply_url(repo: &str, number: u64, kind: ReplyKind, id: u64) -> String {
    match kind {
        ReplyKind::IssueComment => format!("https://github.com/{repo}/issues/{number}#issuecomment-{id}"),
        ReplyKind::ReviewComment => format!("https://github.com/{repo}/pull/{number}#discussion_r{id}"),
    }
}
/// The comment `raw` names on `repo#number`, with its author and URL read from GitHub. A comment of a known kind
/// that GitHub cannot be asked about keeps its URL without the author; a bare ID and a review thread need GitHub.
async fn resolve_reply(app: &App, repo: &str, number: u64, raw: &str) -> Result<ReplyTo> {
    match parse_reply(raw)? {
        ReplyRef::Comment(Some(kind), id) => match read_comment(app, repo, number, kind, id).await {
            Ok(Some(r)) => Ok(r),
            Ok(None) => anyhow::bail!("GitHub has no comment {id} of that kind on {repo}#{number}"),
            Err(e) if e.downcast_ref::<reqwest::Error>().is_some_and(|e| e.status().is_none()) => {
                Ok(ReplyTo { kind, id, author: None, url: reply_url(repo, number, kind, id) })
            }
            Err(e) => Err(e),
        },
        ReplyRef::Comment(None, id) => {
            for kind in [ReplyKind::IssueComment, ReplyKind::ReviewComment] {
                if let Some(r) = read_comment(app, repo, number, kind, id).await? {
                    return Ok(r);
                }
            }
            anyhow::bail!("comment {id} is not a comment of {repo}#{number}: give issuecomment-<id> or discussion_r<id>")
        }
        ReplyRef::Thread(node) => {
            let v = app.github.graphql(format!("query {{ node(id:{}) {{ ... on PullRequestReviewThread {{ pullRequest {{ number repository {{ nameWithOwner }} }} comments(first:1) {{ nodes {{ url author {{ login }} }} }} }} }} }}", json!(node))).await?;
            let t = &v["node"];
            let first = &t["comments"]["nodes"][0];
            let same = t["pullRequest"]["number"].as_u64() == Some(number) && t["pullRequest"]["repository"]["nameWithOwner"].as_str().is_some_and(|r| r.eq_ignore_ascii_case(repo));
            match first["url"].as_str().and_then(|u| comment_of(u).map(|c| (u, c))) {
                Some((url, (kind, id))) if same => Ok(ReplyTo { kind, id, author: first["author"]["login"].as_str().map(str::to_string), url: url.to_string() }),
                _ => anyhow::bail!("review thread {node} is not a thread of {repo}#{number}"),
            }
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionKind {
    Comment,
    Review,
    /// Only actions (`launchpad suggest action`): no text is posted, so there is nothing to insert into GitHub's box,
    /// and Post runs its actions alone (close without a comment, request reviewers…).
    Action,
}

/// What a suggestion also does on its GitHub issue or pull request when the owner posts it: after its comment or
/// review, or alone for an action-only suggestion. Each runs once, in `ItemAction::rank` order; the owner unticks or
/// edits them before Post. A failure is reported on the card, never retried.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ItemAction {
    /// Close the issue or pull request (a pull request without merging). `reason` is for issues only: completed,
    /// not planned, or duplicate of issue `duplicate_of` (its comment then says `Duplicate of #N`, which GitHub links).
    Close {
        #[serde(default)]
        reason: Option<CloseReason>,
        #[serde(default)]
        #[ts(type = "number | null")]
        duplicate_of: Option<u64>,
    },
    /// Reopen a closed issue or pull request.
    Reopen,
    /// Request reviews of a pull request: user logins and team slugs of the repository's organization.
    RequestReviewers {
        #[serde(default)]
        reviewers: Vec<String>,
        #[serde(default)]
        team_reviewers: Vec<String>,
    },
    /// Mark a draft pull request ready for review.
    ReadyForReview,
    /// Convert a pull request to a draft.
    ConvertToDraft,
    /// Update a pull request's branch with its base branch (GitHub's Update branch: a merge commit).
    UpdateBranch,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CloseReason {
    Completed,
    NotPlanned,
    Duplicate,
}
impl ItemAction {
    /// The order Post runs them in: reopen first, close last (after it, a pull request takes no more changes).
    pub fn rank(&self) -> u8 {
        match self {
            ItemAction::Reopen => 0,
            ItemAction::UpdateBranch => 1,
            ItemAction::ConvertToDraft | ItemAction::ReadyForReview => 2,
            ItemAction::RequestReviewers { .. } => 3,
            ItemAction::Close { .. } => 4,
        }
    }
    /// Only pull requests take it.
    pub fn pull_only(&self) -> bool {
        matches!(self, ItemAction::RequestReviewers { .. } | ItemAction::ReadyForReview | ItemAction::ConvertToDraft | ItemAction::UpdateBranch)
    }
    /// Only issues take it (a close reason).
    pub fn issue_only(&self) -> bool {
        matches!(self, ItemAction::Close { reason: Some(_), .. })
    }
    /// What it does, in words ("close as not planned", "request reviews from @a, @org/b").
    pub fn describe(&self) -> String {
        match self {
            ItemAction::Close { reason: None, .. } => "close".into(),
            ItemAction::Close { reason: Some(CloseReason::Completed), .. } => "close as completed".into(),
            ItemAction::Close { reason: Some(CloseReason::NotPlanned), .. } => "close as not planned".into(),
            ItemAction::Close { duplicate_of, .. } => format!("close as duplicate of #{}", duplicate_of.unwrap_or(0)),
            ItemAction::Reopen => "reopen".into(),
            ItemAction::RequestReviewers { reviewers, team_reviewers } => {
                let who: Vec<String> = reviewers.iter().map(|r| format!("@{r}")).chain(team_reviewers.iter().map(|t| format!("team {t}"))).collect();
                format!("request reviews from {}", who.join(", "))
            }
            ItemAction::ReadyForReview => "mark ready for review".into(),
            ItemAction::ConvertToDraft => "convert to draft".into(),
            ItemAction::UpdateBranch => "update the branch".into(),
        }
    }
    fn is_close(&self) -> bool {
        matches!(self, ItemAction::Close { .. })
    }
}
/// The line a duplicate's comment holds, so GitHub links the two issues.
pub fn duplicate_line(of: u64) -> String {
    format!("Duplicate of #{of}")
}
/// Pure: the actions a suggestion may carry. GitHub items only; a review never closes (a comment does); at most one
/// of each kind; not close and reopen, not ready and draft; reviewers are valid logins or team slugs (at most 15); a
/// duplicate names another item; an action-only suggestion has at least one.
pub fn validate_actions(kind: &SuggestionKind, target: &SuggestionTarget, actions: &[ItemAction]) -> Result<()> {
    if actions.is_empty() {
        ensure!(*kind != SuggestionKind::Action, "an action-only suggestion needs at least one action (--close, --reopen, --request-reviewers, --ready-for-review, --convert-to-draft or --update-branch)");
        return Ok(());
    }
    let SuggestionTarget::GitHub { number, .. } = target else { anyhow::bail!("actions are for GitHub issues and pull requests, not Local PRs") };
    for (i, a) in actions.iter().enumerate() {
        ensure!(!actions[..i].iter().any(|b| std::mem::discriminant(a) == std::mem::discriminant(b)), "each action at most once");
    }
    let has = |f: fn(&ItemAction) -> bool| actions.iter().any(f);
    ensure!(!(has(ItemAction::is_close) && has(|a| matches!(a, ItemAction::Reopen))), "close or reopen, not both");
    ensure!(!(has(|a| matches!(a, ItemAction::ReadyForReview)) && has(|a| matches!(a, ItemAction::ConvertToDraft))), "ready for review or convert to draft, not both");
    ensure!(*kind != SuggestionKind::Review || !has(ItemAction::is_close), "a review cannot close: suggest a comment with --close (or `suggest action --close`)");
    for a in actions {
        match a {
            ItemAction::Close { reason, duplicate_of } => {
                ensure!((*reason == Some(CloseReason::Duplicate)) == duplicate_of.is_some(), "a duplicate names its original issue (--duplicate-of <n>)");
                ensure!(duplicate_of.is_none_or(|d| d > 0 && d != *number), "--duplicate-of names another issue of the repository");
            }
            ItemAction::RequestReviewers { reviewers, team_reviewers } => {
                ensure!(!reviewers.is_empty() || !team_reviewers.is_empty(), "name at least one reviewer");
                ensure!(reviewers.len() + team_reviewers.len() <= 15, "at most 15 reviewers");
                for r in reviewers {
                    ensure!(launchpad_core::github::valid_name(r) && !r.contains('.'), "invalid reviewer login {r:?}");
                }
                for t in team_reviewers {
                    ensure!(launchpad_core::github::valid_name(t), "invalid team slug {t:?} (the team's slug, without its organization)");
                }
            }
            _ => {}
        }
    }
    Ok(())
}
/// Pure: the reviewers of `--request-reviewers a,@b,org/team`: logins, and team slugs (`org/team`, the organization
/// left out since it is the repository's).
pub fn split_reviewers(list: &[String]) -> (Vec<String>, Vec<String>) {
    let (mut users, mut teams) = (vec![], vec![]);
    for raw in list.iter().flat_map(|l| l.split(',')).map(|r| r.trim().trim_start_matches('@')).filter(|r| !r.is_empty()) {
        match raw.split_once('/') {
            Some((_, team)) => teams.push(team.to_string()),
            None => users.push(raw.to_string()),
        }
    }
    (users, teams)
}
/// Pure: why these actions do not fit this item (a pull request or an issue), if they do not.
pub fn kind_mismatch(actions: &[ItemAction], pull: bool) -> Option<String> {
    let bad = actions.iter().find(|a| if pull { a.issue_only() } else { a.pull_only() })?;
    Some(if pull { format!("{} is for issues: a pull request closes without a reason", bad.describe()) } else { format!("{} is for pull requests, and this is an issue", bad.describe()) })
}
/// Pure: the actions sorted in the order Post runs them.
fn ranked(mut actions: Vec<ItemAction>) -> Vec<ItemAction> {
    actions.sort_by_key(ItemAction::rank);
    actions
}
/// Pure: what Post does: the owner's list when the window sent one (unticked actions left out, reviewers edited),
/// else the suggestion's own; the older `close` switch adds or removes the close.
fn chosen(s: &Suggestion, actions: Option<Vec<ItemAction>>, close: Option<bool>) -> Vec<ItemAction> {
    let mut list = actions.unwrap_or_else(|| s.actions.clone());
    match close {
        Some(false) => list.retain(|a| !a.is_close()),
        Some(true) if !list.iter().any(ItemAction::is_close) => list.push(ItemAction::Close { reason: None, duplicate_of: None }),
        _ => {}
    }
    ranked(list)
}
/// Pure: a stored suggestion as this version reads it: an older record's `close` becomes its close action, and
/// `close` says whether the actions close.
fn normalized(mut s: Suggestion) -> Suggestion {
    if s.close && !s.actions.iter().any(ItemAction::is_close) {
        s.actions.push(ItemAction::Close { reason: None, duplicate_of: None });
    }
    s.close = s.actions.iter().any(ItemAction::is_close);
    s
}
/// Pure: what the suggestion does, in words, for Today and notifications ("Comment and close", "Close as not
/// planned", "Comment, mark ready for review and request reviews").
pub fn headline(s: &Suggestion) -> String {
    let base = match (&s.kind, &s.merge, s.run_ci) {
        (SuggestionKind::Review, Some(_), _) => "Approve and merge".to_string(),
        (SuggestionKind::Review, None, true) => "Review and run CI".into(),
        (SuggestionKind::Review, None, false) if s.event == "REQUEST_CHANGES" => "Request changes".into(),
        (SuggestionKind::Review, None, false) => "Review".into(),
        (SuggestionKind::Action, ..) => String::new(),
        _ if matches!(&s.target, SuggestionTarget::GitHub { reply_to: Some(_), .. }) => "Reply".into(),
        _ => "Comment".into(),
    };
    let mut parts: Vec<String> = s.actions.iter().map(|a| match a {
        // Short words in a title: "Comment and close", "Comment and request reviews".
        ItemAction::RequestReviewers { .. } => "request reviews".into(),
        ItemAction::Close { reason: Some(CloseReason::Duplicate), .. } => "close as duplicate".into(),
        a => a.describe(),
    }).collect();
    if !base.is_empty() {
        parts.insert(0, base);
    }
    capitalized(&match parts.len() {
        0 => "Suggestion".to_string(),
        1 => parts.remove(0),
        n => format!("{} and {}", parts[..n - 1].join(", "), parts[n - 1]),
    })
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
    /// A comment on a GitHub issue or pull request the agent recommends posting and then closing the item
    /// (`--close`): Post closes it by default (Comment and close); the owner may post without closing. Kept in step with
    /// `actions` (whether they close); older records have only this.
    #[serde(default)]
    pub close: bool,
    /// What Post also does on the item (`ItemAction`), in the order it runs them.
    #[serde(default)]
    pub actions: Vec<ItemAction>,
    /// What failed after the review was posted (the merge, the workflow approval or an action). Never retried.
    #[serde(default)]
    pub followup_error: Option<String>,
}
#[derive(Deserialize)]
pub struct Create {
    pub target: SuggestionTarget,
    /// `--reply-to`: the comment this comment answers (see `parse_reply`); Home resolves it into the target.
    #[serde(default)]
    pub reply_to: Option<String>,
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
    /// `--close`: the agent recommends closing the item after this comment (the same as a close action).
    #[serde(default)]
    pub close: bool,
    /// The actions Post also runs (`--close`, `--reopen`, `--request-reviewers`…).
    #[serde(default)]
    pub actions: Vec<ItemAction>,
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
    /// Whether Post closes the item by default; absent keeps it (the extension's edit changes only the text).
    #[serde(default)]
    pub close: Option<bool>,
    /// The actions Post runs (the owner unticks or edits them); absent keeps them.
    #[serde(default)]
    pub actions: Option<Vec<ItemAction>>,
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
    if *kind == SuggestionKind::Action {
        ensure!(body.trim().is_empty() && event == "COMMENT", "an action-only suggestion has no text: suggest a comment with these actions instead");
        return Ok(());
    }
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
                .map(|s| Ok(normalized(serde_json::from_str(&s)?)))
                .collect()
        })
        .await
}
/// Home: a Suggestion of the session that the owner has not posted yet (a draft, a claimed or uncertain post, one
/// inserted on GitHub but not seen posted), in words; None when every one is posted. Discarded ones are deleted.
pub async fn waiting_for(app: &App, session_id: &str) -> Result<Option<String>> {
    Ok(list(app).await?.into_iter().find(|s| s.session_id.as_deref() == Some(session_id) && !is_posted(&s.status)).map(|s| format!("Suggestion {} is {}", s.id, s.status)))
}

/// Pure: the owner posted it (from Launchpad or by hand).
pub fn is_posted(status: &str) -> bool {
    matches!(status, "posted" | "posted_by_hand")
}

pub(crate) async fn get(app: &App, id: &str) -> Result<Suggestion> {
    list(app)
        .await?
        .into_iter()
        .find(|s| s.id == id)
        .context("suggestion not found")
}
pub(crate) async fn save(app: &App, s: &Suggestion) -> Result<()> {
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
/// Pure: the actions of a new suggestion (`--close` included) and its kind and text: a close as duplicate needs the
/// `Duplicate of #N` line GitHub links, so an action-only one becomes a comment of that line, and a comment without it
/// starts with it.
fn prepare_actions(q: &mut Create) {
    if q.close && !q.actions.iter().any(ItemAction::is_close) {
        q.actions.push(ItemAction::Close { reason: None, duplicate_of: None });
    }
    q.actions = ranked(std::mem::take(&mut q.actions));
    let dup = q.actions.iter().find_map(|a| match a {
        ItemAction::Close { duplicate_of: Some(n), .. } => Some(*n),
        _ => None,
    });
    if let Some(n) = dup {
        let line = duplicate_line(n);
        if q.kind == SuggestionKind::Action {
            q.kind = SuggestionKind::Comment;
            q.body = line;
        } else if q.kind == SuggestionKind::Comment && !q.body.to_lowercase().contains(&line.to_lowercase()) {
            q.body = format!("{line}\n\n{}", q.body);
        }
    }
    q.close = q.actions.iter().any(ItemAction::is_close);
}
/// Whether item `n` is a pull request, read from GitHub now; None when GitHub cannot be asked (the actions then
/// fail at Post if they do not fit).
async fn is_pull(app: &App, repo: &str, n: u64) -> Result<Option<bool>> {
    match items::issue(app, repo, n).await {
        Ok(v) => Ok(Some(v["pull_request"].is_object())),
        Err(e) if e.downcast_ref::<reqwest::Error>().and_then(|e| e.status()) == Some(reqwest::StatusCode::NOT_FOUND) => anyhow::bail!("GitHub has no issue or pull request {repo}#{n}"),
        Err(e) => {
            tracing::warn!("Suggestion actions: reading {repo}#{n}: {e:#}");
            Ok(None)
        }
    }
}
pub async fn create(app: &App, mut q: Create) -> Result<Suggestion> {
    prepare_actions(&mut q);
    validate(&q.body, &q.event, &q.kind)?;
    if let Some(raw) = q.reply_to.take().filter(|r| !r.trim().is_empty()) {
        ensure!(q.kind == SuggestionKind::Comment, "only comments can reply to a comment (--reply-to)");
        let SuggestionTarget::GitHub { repo, number, reply_to } = &mut q.target else { anyhow::bail!("--reply-to needs --repo and --number") };
        items::validate(repo)?;
        *reply_to = Some(resolve_reply(app, repo, *number, &raw).await?);
    }
    if let SuggestionTarget::GitHub { reply_to: Some(_), .. } = &q.target {
        ensure!(q.kind == SuggestionKind::Comment, "only comments can reply to a comment");
    }
    validate_comments(&q.kind, &q.comments)?;
    validate_followups(&q.kind, &q.event, &q.target, q.merge.as_deref(), q.run_ci)?;
    validate_actions(&q.kind, &q.target, &q.actions)?;
    match &q.target {
        SuggestionTarget::GitHub { repo, number, .. } => {
            items::validate(repo)?;
            ensure!(*number > 0, "invalid item number");
            if q.actions.iter().any(|a| a.pull_only() || a.issue_only())
                && let Some(pull) = is_pull(app, repo, *number).await?
                && let Some(why) = kind_mismatch(&q.actions, pull)
            {
                anyhow::bail!(why);
            }
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
        close: q.close,
        actions: q.actions,
        followup_error: None,
    };
    save(app, &s).await?;
    if let (Some(session), SuggestionTarget::GitHub { repo, number, .. }) =
        (s.session_id.as_deref(), &s.target)
        && let Err(e) = launchpad_core::ref_use::link(app, session, repo, *number, None).await
    {
        tracing::warn!("Suggestion item link: {e:#}");
    }
    crate::ledger::drafted(app, &s).await;
    Ok(s)
}
pub async fn edit(app: &App, id: &str, q: Edit) -> Result<Suggestion> {
    let lock = app.suggestion_locks.get(id);
    let _guard = lock.lock().await;
    let mut s = get(app, id).await?;
    ensure!(s.status == "draft", "only unposted drafts can be edited");
    validate(&q.body, &q.event, &s.kind)?;
    validate_followups(&s.kind, &q.event, &s.target, q.merge.as_deref(), q.run_ci)?;
    if q.actions.is_some() || q.close.is_some() {
        let actions = chosen(&s, q.actions, q.close);
        validate_actions(&s.kind, &s.target, &actions)?;
        s.close = actions.iter().any(ItemAction::is_close);
        s.actions = actions;
    }
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
    ensure!(s.status != "posted" && s.status != "posted_by_hand", "suggestion is already posted");
    crate::ledger::discarded(app, &s).await;
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
/// The owner's Post: the comment or review, then its follow-ups once: a review's merge or workflow approval, then the
/// actions (`actions`: the owner's list, unticked ones left out; absent, the suggestion's own; `close` adds or
/// removes the close, Comment and close). An action-only suggestion runs its actions alone.
pub async fn post(app: &App, id: &str, close: Option<bool>, actions: Option<Vec<ItemAction>>) -> Result<Suggestion> {
    let lock = app.suggestion_locks.get(id);
    let _guard = lock.lock().await;
    let mut s = get(app, id).await?;
    ensure!(s.status != "inserted", "inserted on GitHub: post it there with GitHub's own button, or Discard it here");
    ensure!(s.status != "posted_by_hand", "already posted on GitHub by you");
    ensure!(
        s.status == "draft",
        "publication already attempted; check GitHub before discarding or creating another suggestion"
    );
    let actions = chosen(&s, actions, close);
    validate_actions(&s.kind, &s.target, &actions)?;
    ensure!(s.kind != SuggestionKind::Action || !actions.is_empty(), "every action is unticked: Discard the suggestion instead");
    s.close = actions.iter().any(ItemAction::is_close);
    s.actions = actions;
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
        SuggestionTarget::GitHub { repo, number, reply_to } => {
            if s.kind == SuggestionKind::Review {
                let current = items::pull(app, repo, *number).await.context("reviews require a pull request")?;
                if current["head"]["sha"].as_str() != s.head.as_deref() {
                    crate::ledger::outdated(app, &s).await;
                    anyhow::bail!("PR head changed; ask the agent for a fresh review");
                }
                ensure!(current["state"] == "open", "pull request is no longer open");
            }
            s.status = "posting".into();
            save(app, &s).await?;
            if s.kind == SuggestionKind::Action {
                let (repo, number) = (repo.clone(), *number);
                return run_actions_only(app, s, repo, number).await;
            }
            // The agent's attribution line ends what reaches GitHub; the draft keeps the text as written. Post is
            // only ever the owner's click (never an agent's or admin action), so it is a manual publication.
            let text = format!("{}{}", reply_to.as_ref().map(ReplyTo::prefix).unwrap_or_default(), s.body);
            let body = match launchpad_core::attribution::github_line(app, s.session_id.as_deref(), launchpad_core::attribution::Origin::Manual) {
                Some(line) => launchpad_core::attribution::sign(&text, &line, launchpad_core::attribution::Separator::Rule),
                None => text,
            };
            let result = match reply_to {
                // A review thread's reply joins the thread (GitHub's review-comment reply API).
                Some(r) if r.kind == ReplyKind::ReviewComment => items::reply(app, repo, *number, r.id, &body).await,
                _ if s.kind == SuggestionKind::Review => items::review(app, repo, *number, &json!({"body": body, "event": s.event, "commit_id": s.head, "comments": s.comments})).await,
                _ => items::comment(app, repo, *number, &body).await,
            };
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
    crate::ledger::posted_by_launchpad(app, &s).await;
    if let SuggestionTarget::GitHub { repo, number, .. } = &s.target {
        // The follow-ups run once, after the review is saved as posted: a failure stays on the card, never retried.
        let head = s.head.clone().unwrap_or_default();
        let mut errors = vec![];
        if s.run_ci {
            errors.extend(items::approve_runs(app, repo, *number, &head).await.err().map(|e| format!("Running the workflows failed: {e:#}")));
        } else if let Some(method) = &s.merge {
            errors.extend(items::merge(app, repo, *number, &head, method).await.err().map(|e| format!("The merge failed: {e:#}. Check the pull request on GitHub before merging it there.")));
        }
        errors.extend(apply_actions(app, repo, *number, &s.actions).await);
        if !errors.is_empty() {
            let error = errors.join(" ");
            s.followup_error = Some(error.clone());
            save(app, &s).await?;
            anyhow::bail!("The {} was posted. {error}", if s.kind == SuggestionKind::Review { "review" } else { "comment" });
        }
    }
    Ok(s)
}
/// Runs `actions` on `repo#number` in order, each once: a sentence per failure.
async fn apply_actions(app: &App, repo: &str, number: u64, actions: &[ItemAction]) -> Vec<String> {
    let mut errors = vec![];
    for a in actions {
        if let Err(e) = items::apply(app, repo, number, a).await {
            errors.push(format!("{} failed: {e:#}. Do it on GitHub.", capitalized(&a.describe())));
        }
    }
    errors
}
fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}
/// Post of an action-only suggestion, claimed (`posting`) by the caller: its actions run once. When none went
/// through: a draft again when GitHub refused each one for sure (nothing changed), else uncertain (check GitHub);
/// otherwise it is posted, with the failures on its card. Never retried.
async fn run_actions_only(app: &App, mut s: Suggestion, repo: String, number: u64) -> Result<Suggestion> {
    let (mut done, mut uncertain, mut errors) = (0, false, vec![]);
    for a in &s.actions {
        match items::apply(app, &repo, number, a).await {
            Ok(()) => done += 1,
            Err(e) => {
                uncertain |= launchpad_core::github::delivery_uncertain(&e);
                errors.push(format!("{} failed: {e:#}.", capitalized(&a.describe())));
            }
        }
    }
    s.url = Some(format!("https://github.com/{repo}/issues/{number}"));
    if done == 0 {
        s.status = if uncertain { "uncertain" } else { "draft" }.into();
        save(app, &s).await?;
        anyhow::bail!("{} {}", errors.join(" "), if uncertain { "It may have gone through: check GitHub; it is not retried." } else { "Nothing changed on GitHub." });
    }
    s.status = "posted".into();
    if !errors.is_empty() {
        s.followup_error = Some(format!("{} Do it on GitHub; it is not retried.", errors.join(" ")));
    }
    save(app, &s).await?;
    match &s.followup_error {
        Some(e) => anyhow::bail!("Done in part. {e}"),
        None => Ok(s),
    }
}
/// Included in every launched session, with the exact daemon and session identity (followed by the connections'
/// instruction, when the owner has some).
pub fn instruction(app: &App, session: &str) -> String {
    format!(
        "Agent-written GitHub comments and reviews must stay local. Use the launchpad skill and `{0} suggest comment|review --repo <owner/repo> --number <n> --body-file <file> --session-id {session}`. Reviews also require --head <inspected-40-character-commit> and optionally --event approve|request-changes, and may add --comments-file <file.json> with inline line comments posted in the same review (a JSON array of {{path, line, side: RIGHT or LEFT for a removed line, start_line (optional, for a range), body}}; a body may hold a ```suggestion block); put each finding about specific lines in a line comment and keep the --body-file for the overall summary; an approving review may add --merge squash|merge|rebase (the owner's Post click then merges at that head), and a review may add --run-ci (Post also approves the head's GitHub Actions runs that wait for a maintainer). A comment that answers one comment adds --reply-to <issuecomment-<id>, discussion_r<id> or the comment's URL> (a review thread's comment gets its reply in that thread). Comments and reviews of a GitHub item take action flags that the owner's same Post click applies after the text (each one can be unticked before Post): --close (a pull request closes without merging; an issue may add a reason, --close completed or --close not-planned), --duplicate-of <n> (closes an issue as a duplicate; the comment starts with `Duplicate of #n`), --reopen, and for pull requests --request-reviewers <login,org/team>, --ready-for-review, --convert-to-draft and --update-branch (a review never closes). An action without any text is `{0} suggest action --repo <owner/repo> --number <n> <flags> --session-id {session}` (a close without a comment, a review request). File every action you recommend (close, reopen, reviewers, draft state, branch update, merge, CI run, request changes, reply) as a suggestion with the matching flag, never only as prose or a quoted draft in a message or note: the owner acts on it with one click. With propose or admin Launchpad CLI access you may approve a pull request's waiting GitHub Actions runs yourself, without the owner's click, with `{0} ci approve --repo <owner/repo> --number <n> --head <inspected-40-character-commit> --session-id {session}`, only after reading the whole diff at that head, and never when it adds or changes workflows, actions or other CI configuration (Home refuses those too) or looks like spam or malicious (for example a pull request that adds a workflow and changes SECURITY.md's security contact): then suggest a comment and leave CI to the owner. Home records each approval as a private note on the item. Saving a suggestion links this session to the item and its repository#number workspace topic. Use `launchpad link --repo <owner/repo> --number <n> --session-id <id>` to link without a suggestion (optional --comment-url). Local PR bodies with Fixes #N or Closes #N link their session and show the Local PR on the issue in the extension. For local PRs use --predraft <id> instead of --repo/--number. Never publish suggestions yourself; the owner edits and posts with a click.",
        app.paths.cli_command()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn followups_need_an_approving_github_review() {
        let gh = SuggestionTarget::GitHub { repo: "o/r".into(), number: 1, reply_to: None };
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

    fn close(reason: Option<CloseReason>, duplicate_of: Option<u64>) -> ItemAction {
        ItemAction::Close { reason, duplicate_of }
    }

    #[test]
    fn actions_fit_github_items_and_each_other() {
        let gh = SuggestionTarget::GitHub { repo: "o/r".into(), number: 1, reply_to: None };
        let local = SuggestionTarget::Predraft { id: "p".into() };
        let (comment, review, action) = (SuggestionKind::Comment, SuggestionKind::Review, SuggestionKind::Action);
        let reviewers = |r: &[&str], t: &[&str]| ItemAction::RequestReviewers { reviewers: r.iter().map(|x| x.to_string()).collect(), team_reviewers: t.iter().map(|x| x.to_string()).collect() };
        assert!(validate_actions(&comment, &gh, &[close(None, None)]).is_ok());
        assert!(validate_actions(&action, &gh, &[close(Some(CloseReason::NotPlanned), None)]).is_ok());
        assert!(validate_actions(&review, &gh, &[close(None, None)]).is_err(), "a review never closes");
        assert!(validate_actions(&comment, &local, &[close(None, None)]).is_err(), "a Local PR has nothing to close");
        assert!(validate_actions(&review, &local, &[]).is_ok());
        assert!(validate_actions(&action, &gh, &[]).is_err(), "an action-only suggestion needs an action");
        assert!(validate_actions(&comment, &gh, &[close(None, None), ItemAction::Reopen]).is_err());
        assert!(validate_actions(&comment, &gh, &[ItemAction::ReadyForReview, ItemAction::ConvertToDraft]).is_err());
        assert!(validate_actions(&comment, &gh, &[ItemAction::Reopen, ItemAction::Reopen]).is_err());
        assert!(validate_actions(&review, &gh, &[ItemAction::ConvertToDraft, reviewers(&["ana"], &["core"]), ItemAction::UpdateBranch]).is_ok());
        assert!(validate_actions(&comment, &gh, &[reviewers(&[], &[])]).is_err());
        assert!(validate_actions(&comment, &gh, &[reviewers(&["no/slash"], &[])]).is_err());
        assert!(validate_actions(&comment, &gh, &[close(Some(CloseReason::Duplicate), Some(9))]).is_ok());
        assert!(validate_actions(&comment, &gh, &[close(Some(CloseReason::Duplicate), None)]).is_err());
        assert!(validate_actions(&comment, &gh, &[close(Some(CloseReason::Duplicate), Some(1))]).is_err(), "not a duplicate of itself");
        assert!(validate_actions(&comment, &gh, &[close(None, Some(9))]).is_err());
        assert_eq!(kind_mismatch(&[ItemAction::ReadyForReview], false).as_deref(), Some("mark ready for review is for pull requests, and this is an issue"));
        assert!(kind_mismatch(&[close(Some(CloseReason::NotPlanned), None)], true).is_some());
        assert!(kind_mismatch(&[close(None, None), ItemAction::Reopen], true).is_none());
        assert_eq!(split_reviewers(&["ana, @bo".into(), "org/core".into()]), (vec!["ana".to_string(), "bo".into()], vec!["core".to_string()]));
    }

    #[test]
    fn stored_suggestions_keep_their_actions_and_older_ones_their_close() {
        // An older record without actions: `close` reads as its close action; one without either closes nothing.
        let old = json!({"id": "s", "target": {"kind": "git_hub", "repo": "o/r", "number": 1}, "kind": "comment", "body": "Closing: spam.", "event": "COMMENT", "head": null, "author": "Claude Code", "session_id": null, "created_at": 0, "status": "draft", "url": null});
        let s = normalized(serde_json::from_value(old.clone()).unwrap());
        assert!(!s.close && s.actions.is_empty());
        let mut closing = old.clone();
        closing["close"] = json!(true);
        let s = normalized(serde_json::from_value(closing).unwrap());
        assert!(s.close);
        assert_eq!(s.actions, vec![close(None, None)]);
        // Every action round-trips through the stored JSON.
        let mut s = s;
        s.actions = vec![ItemAction::Reopen, ItemAction::UpdateBranch, ItemAction::ReadyForReview, ItemAction::RequestReviewers { reviewers: vec!["ana".into()], team_reviewers: vec![] }, close(Some(CloseReason::Duplicate), Some(4))];
        let back = normalized(serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap());
        assert_eq!(back.actions, s.actions);
        assert!(back.close);
        assert_eq!(serde_json::to_value(&s.actions[4]).unwrap(), json!({"kind": "close", "reason": "duplicate", "duplicate_of": 4}));
        let q: Create = serde_json::from_value(json!({"target": {"kind": "git_hub", "repo": "o/r", "number": 1}, "kind": "action", "body": "", "actions": [{"kind": "convert_to_draft"}]})).unwrap();
        assert_eq!((q.kind, q.actions), (SuggestionKind::Action, vec![ItemAction::ConvertToDraft]));
        let e: Edit = serde_json::from_value(json!({"body": "x", "event": "COMMENT"})).unwrap();
        assert_eq!((e.close, e.actions), (None, None), "an edit without them keeps them");
    }

    #[test]
    fn post_runs_what_the_owner_left_in_order() {
        let mut s = normalized(serde_json::from_value(json!({"id": "s", "target": {"kind": "git_hub", "repo": "o/r", "number": 1}, "kind": "comment", "body": "x", "event": "COMMENT", "head": null, "author": "A", "session_id": null, "created_at": 0, "status": "draft", "url": null})).unwrap());
        s.actions = vec![close(None, None), ItemAction::RequestReviewers { reviewers: vec!["ana".into()], team_reviewers: vec![] }, ItemAction::Reopen];
        // Absent: the suggestion's own, reopen first and close last.
        assert_eq!(chosen(&s, None, None).iter().map(ItemAction::rank).collect::<Vec<_>>(), vec![0, 3, 4]);
        // The older Comment and close switch: without closing, or adding the close.
        assert!(!chosen(&s, None, Some(false)).iter().any(ItemAction::is_close));
        assert_eq!(chosen(&s, Some(vec![]), Some(true)), vec![close(None, None)]);
        // The owner's list replaces the agent's (unticked ones left out).
        assert_eq!(chosen(&s, Some(vec![ItemAction::Reopen]), None), vec![ItemAction::Reopen]);
    }

    #[test]
    fn duplicates_say_so_in_their_comment() {
        let create = |kind: &str, body: &str, actions: serde_json::Value| -> Create { serde_json::from_value(json!({"target": {"kind": "git_hub", "repo": "o/r", "number": 1}, "kind": kind, "body": body, "actions": actions})).unwrap() };
        let mut q = create("action", "", json!([{"kind": "close", "reason": "duplicate", "duplicate_of": 5}]));
        prepare_actions(&mut q);
        assert_eq!((q.kind.clone(), q.body.as_str(), q.close), (SuggestionKind::Comment, "Duplicate of #5", true));
        let mut q = create("comment", "Same bug as the other one.", json!([{"kind": "close", "reason": "duplicate", "duplicate_of": 5}]));
        prepare_actions(&mut q);
        assert_eq!(q.body, "Duplicate of #5\n\nSame bug as the other one.");
        let mut q = create("comment", "duplicate of #5, thanks!", json!([{"kind": "close", "reason": "duplicate", "duplicate_of": 5}]));
        prepare_actions(&mut q);
        assert_eq!(q.body, "duplicate of #5, thanks!");
        // `--close` alone is a close action.
        let mut q = create("comment", "Done.", json!([]));
        q.close = true;
        prepare_actions(&mut q);
        assert_eq!(q.actions, vec![close(None, None)]);
    }

    #[test]
    fn headlines_name_every_action() {
        let mut s = normalized(serde_json::from_value(json!({"id": "s", "target": {"kind": "git_hub", "repo": "o/r", "number": 1}, "kind": "comment", "body": "x", "event": "COMMENT", "head": null, "author": "A", "session_id": null, "created_at": 0, "status": "draft", "url": null, "close": true})).unwrap());
        assert_eq!(headline(&s), "Comment and close");
        s.actions = vec![ItemAction::ReadyForReview, ItemAction::RequestReviewers { reviewers: vec!["a".into()], team_reviewers: vec![] }];
        assert_eq!(headline(&s), "Comment, mark ready for review and request reviews");
        s.kind = SuggestionKind::Action;
        s.actions = vec![close(Some(CloseReason::NotPlanned), None)];
        assert_eq!(headline(&s), "Close as not planned");
        s.kind = SuggestionKind::Review;
        s.event = "REQUEST_CHANGES".into();
        s.actions = vec![ItemAction::ConvertToDraft];
        assert_eq!(headline(&s), "Request changes and convert to draft");
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

#[cfg(test)]
mod waiting_tests {
    #[test]
    fn only_posted_suggestions_are_done() {
        for status in ["posted", "posted_by_hand"] {
            assert!(super::is_posted(status));
        }
        for status in ["draft", "posting", "uncertain", "inserted"] {
            assert!(!super::is_posted(status));
        }
    }
}

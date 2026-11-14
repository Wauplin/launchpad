//! The Chrome extension's page context (`GET /api/ext/context`) and row states (`GET /api/ext/rows`): what Launchpad
//! knows about a github.com page, answered from Home's records and cache.db only (never a GitHub API call on the
//! request path), in one call per navigation. The extension's device is the owner, but each answer is narrowed to one
//! workspace: the folder's for a repository in Launchpad, else the one the extension shows.
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Result;
use launchpad_core::App;
use launchpad_core::model::{DiffSelection, Project, Session, SessionStatus};
use launchpad_core::predrafts::Predraft;
use launchpad_core::ref_use::Use;
use launchpad_core::today::Today;
use launchpad_core::workspace::Workspace;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::suggestions::{ReplyTo, Suggestion, SuggestionKind, SuggestionTarget};

/// The kinds of GitHub pages the extension recognizes, from the URL alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "ExtPageKind")]
pub enum PageKind {
    /// The repository's Code tab (also its tree and blob pages).
    Repo,
    Pulls,
    Issues,
    PrConversation,
    PrFiles,
    PrCommits,
    Issue,
    Notifications,
    ActionsRun,
    Compare,
    NewRelease,
    Other,
}

impl PageKind {
    pub fn is_pr(self) -> bool {
        matches!(self, PageKind::PrConversation | PageKind::PrFiles | PageKind::PrCommits)
    }
}

/// A github.com page: its kind and what its URL names.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ExtPage {
    pub kind: PageKind,
    /// `owner/name` as written in the URL.
    pub repo: Option<String>,
    /// The issue or pull request.
    #[ts(type = "number | null")]
    pub number: Option<u64>,
    /// An Actions run.
    #[ts(type = "number | null")]
    pub run: Option<u64>,
    /// From Home's cache of the item (null when Home has not fetched it): its title, and for a pull request its head
    /// commit and branch.
    pub title: Option<String>,
    pub head: Option<String>,
    pub head_branch: Option<String>,
    /// A pull request's branch repository (`owner/name`), from Home's cache.
    pub head_repo: Option<String>,
    /// The pull request's author lets maintainers push to its branch (null: unknown, or the same repository).
    pub maintainer_can_modify: Option<bool>,
}

/// GitHub's top-level paths that are not owners.
const RESERVED: &[&str] = &[
    "about", "account", "apps", "codespaces", "collections", "contact", "customer-stories", "dashboard", "enterprise", "enterprises", "explore", "features",
    "issues", "login", "logout", "marketplace", "new", "notifications", "organizations", "orgs", "pricing", "pulls", "readme", "search", "security",
    "sessions", "settings", "signup", "site", "sponsors", "stars", "team", "topics", "trending", "users", "watching",
];

/// The page of a github.com URL. Anything else (another host, a malformed path) is `Other` without a repository.
pub fn parse_page(url: &str) -> ExtPage {
    let mut page = ExtPage { kind: PageKind::Other, repo: None, number: None, run: None, title: None, head: None, head_branch: None, head_repo: None, maintainer_can_modify: None };
    let Some(rest) = url.strip_prefix("https://github.com/").or_else(|| url.strip_prefix("https://www.github.com/")) else { return page };
    let path = rest.split(['?', '#']).next().unwrap_or("");
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    match parts.as_slice() {
        ["notifications", ..] => {
            page.kind = PageKind::Notifications;
            return page;
        }
        [owner, name, rest @ ..] if !RESERVED.contains(&owner.to_ascii_lowercase().as_str()) => {
            let full = format!("{owner}/{}", name.strip_suffix(".git").unwrap_or(name));
            if !launchpad_core::github::valid_repo(&full) {
                return page;
            }
            page.repo = Some(full);
            let num = |s: &&str| s.parse::<u64>().ok().filter(|n| *n > 0);
            page.kind = match rest {
                [] | ["tree", ..] | ["blob", ..] => PageKind::Repo,
                ["pulls", ..] => PageKind::Pulls,
                ["issues"] => PageKind::Issues,
                ["issues", n, ..] if num(n).is_some() => {
                    page.number = num(n);
                    PageKind::Issue
                }
                ["issues", ..] => PageKind::Issues,
                ["pull", n, tail @ ..] if num(n).is_some() => {
                    page.number = num(n);
                    match tail {
                        ["files" | "changes", ..] => PageKind::PrFiles,
                        ["commits", ..] => PageKind::PrCommits,
                        _ => PageKind::PrConversation,
                    }
                }
                ["actions", "runs", n, ..] if num(n).is_some() => {
                    page.run = num(n);
                    PageKind::ActionsRun
                }
                ["compare", ..] => PageKind::Compare,
                ["releases", "new", ..] => PageKind::NewRelease,
                _ => PageKind::Other,
            };
        }
        _ => {}
    }
    page
}

/// How a folder relates to the page's repository.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "ExtFolderRelation")]
pub enum Relation {
    /// The folder's `origin` is this repository.
    Origin,
    /// The folder's `upstream` is this repository (the folder is a clone of a fork of it).
    Upstream,
    /// This repository is a fork of the folder's repository (the owner's fork, or a fork Home knows from its cache).
    Fork,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ExtFolder {
    pub id: String,
    pub name: String,
    pub workspace_id: String,
    pub workspace: String,
    pub relation: Relation,
    /// The folder's own repository (`origin`).
    pub repo: Option<String>,
    pub upstream: Option<String>,
    /// The page's repository belongs to the GitHub account Home is signed in with (a fork: "Your fork of …").
    pub yours: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ExtWorkspace {
    pub id: String,
    pub name: String,
}

/// An agent that works on the page's item (or repository).
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ExtAgent {
    pub session_id: String,
    pub title: String,
    pub status: SessionStatus,
    pub needs_you: bool,
    pub open_question: bool,
    /// The bot whose thread it is.
    pub bot_id: Option<String>,
    pub bot: Option<String>,
    /// The bot's thread it belongs to (`BotLaunch.thread`, `main` for older sessions).
    pub thread: Option<String>,
    /// The issue or pull request it is about (null: the repository or its folder).
    #[ts(type = "number | null")]
    pub number: Option<u64>,
    #[ts(type = "number")]
    pub since: i64,
    /// The comment it was last asked about on this item (`issuecomment-123`, `discussion_r45`): its live card goes
    /// under that comment.
    pub comment: Option<String>,
    /// Start of its last answer, one line.
    pub last_message: Option<String>,
}

/// A suggestion (agent-written comment or review) waiting to be posted.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ExtSuggestion {
    pub id: String,
    pub kind: SuggestionKind,
    #[ts(type = "number")]
    pub number: u64,
    /// The commit it was written for.
    pub head: Option<String>,
    pub comments: u32,
    /// Line comments written for another commit than the pull request's head in Home's cache.
    pub outdated: u32,
    /// `draft`, `inserted` (Insert put it into GitHub's box; GitHub's button posts it), `posting` or `uncertain`.
    pub status: String,
    pub session_id: Option<String>,
    /// The bot whose session wrote it.
    pub bot: Option<String>,
    /// The comment it answers (a thread reply goes into that comment's reply box).
    pub reply_to: Option<ReplyTo>,
    /// The text Insert puts into GitHub's box: the body, after the link line of an issue-comment reply.
    pub text: String,
    /// The attribution line Insert adds after a blank line (the session's, like Launchpad's own Post).
    pub attribution: String,
    /// A review's event (`COMMENT`, `APPROVE`, `REQUEST_CHANGES`), shown next to its summary.
    pub event: String,
    /// A review's line comments (the extension's ghost comments on Files changed).
    pub lines: Vec<ExtLine>,
    #[ts(type = "number")]
    pub created_at: i64,
    /// The bot's name, else the session's title, else the agent CLI.
    pub author: String,
    /// The text as written (without an issue-comment reply's link line): what Edit changes.
    pub body: String,
}

/// One line comment of a suggested review, with what the owner did with it so far.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ExtLine {
    /// Its ledger part: `line:<path>:<line>:<side>`.
    pub part: String,
    pub path: String,
    #[ts(type = "number")]
    pub line: u64,
    #[ts(type = "number | null")]
    pub start_line: Option<u64>,
    /// `RIGHT` or `LEFT`.
    pub side: String,
    pub body: String,
    /// The owner's last step with it (`inserted`, `posted_by_hand`, `confirmed`), from the feedback ledger; null: none.
    pub state: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ExtLocalPr {
    pub id: String,
    pub title: String,
    pub branch: String,
    pub base_branch: String,
    pub session_id: Option<String>,
    /// `open` (to publish), `published`, `merged` or `discarded`.
    pub state: String,
    #[ts(type = "number | null")]
    pub published_number: Option<u64>,
    pub published_url: Option<String>,
    /// Who wrote it: the bot of its session, else the session's title.
    pub author: Option<String>,
    /// Review notes waiting for its agent.
    pub notes: u32,
    /// The other open Local PRs of its change set (published together with Publish all, in Launchpad).
    pub set: u32,
    /// A publication started at this commit and did not finish (Launchpad's page offers Retry or Unlock).
    pub pushed_head: Option<String>,
}

/// A bot's (or an agent's) last report about an issue or pull request (`launchpad propose report`).
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ExtReport {
    /// The proposal card.
    pub id: String,
    /// `needs_you`, `merged`, `approved`, `commented`, `waiting`, `closed` or `info`.
    pub outcome: String,
    pub bot: Option<String>,
    pub title: String,
    #[ts(type = "number")]
    pub at: i64,
    /// Still waiting in Today (not read or answered yet).
    pub waiting: bool,
}

/// A preset the owner may add to a message to a bot (`bots: true` in Settings → Presets).
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ExtPreset {
    pub id: String,
    pub label: String,
    pub text: String,
}

/// What Review with agent uses on a repository's lists: the bot and preset the owner chose there last
/// (`github.review-choice`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, TS)]
pub struct ExtReviewChoice {
    pub bot: Option<String>,
    pub preset: Option<String>,
}

/// An agent's Launchpad branch that holds the pull request's head plus `ahead` commits: Push to this PR
/// (`branch.push-to-pr`) can send it as a fast-forward.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ExtPushReady {
    pub branch: String,
    pub ahead: u32,
    /// The branch's commit on Home: the one the push sends (the action's `head`).
    pub head: String,
    /// The agent whose branch it is.
    pub session_id: Option<String>,
    pub title: Option<String>,
}

/// A Launchpad branch Home pushes to a pull request on GitHub (`git_follow`).
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ExtBranch {
    pub branch: String,
    /// The pull request.
    pub url: String,
    pub head: Option<String>,
    /// Home has commits GitHub does not have yet ("Not on GitHub yet").
    pub not_on_github: bool,
    /// Why Home's last push failed.
    pub error: Option<String>,
    /// An agent branch Home does not push there yet, with the commits it would add (`branch.push-to-pr`).
    pub push_ready: Option<ExtPushReady>,
}

/// The Inbox thread of the page's item.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ExtInbox {
    pub thread: String,
    pub unread: bool,
    pub reason: String,
}

/// The default bot of the extension's Ask on this page, and why it is the one.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ExtBot {
    pub id: String,
    pub name: String,
    pub slug: String,
    /// `folder` (the folder's setting), `repository` (the last bot used on it) or `ask` (the workspace's Ask bot).
    pub source: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, TS)]
pub struct ExtDefaults {
    pub bot: Option<ExtBot>,
    /// The bot and preset of the repository's last Review with agent from its lists (null: none yet).
    pub review: Option<ExtReviewChoice>,
}

/// A private note about the page's item (`crate::notes`): one entry of the item's private timeline.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ExtNote {
    pub id: String,
    /// The bot's name, else the session's title ("You" for the owner's).
    pub author: String,
    pub bot_id: Option<String>,
    pub session_id: Option<String>,
    /// The owner's own private note.
    pub by_owner: bool,
    /// Markdown.
    pub body: String,
    #[ts(type = "number")]
    pub created_at: i64,
    /// A pull request's head when it was written.
    pub head: Option<String>,
    /// The note it answers.
    pub reply_to: Option<String>,
    /// The suggested comment it carries (the waiting ones are also in `ExtContext.suggestions`, with their text).
    pub suggestion: Option<ExtNoteSuggestion>,
}

/// The state of a note's suggested comment.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ExtNoteSuggestion {
    pub id: String,
    /// `draft`, `inserted`, `posting`, `uncertain`, `posted`, `posted_by_hand`, or `discarded` (gone).
    pub status: String,
    /// The posted comment.
    pub url: Option<String>,
}

/// What a pull request changes in files that decide what its CI runs (workflows, scripts, dependencies, build
/// files), from Home's cached files of its head: a note next to GitHub's Approve and run.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ExtCi {
    /// The head these files are of.
    pub head: String,
    /// Files changed in all.
    pub files: u32,
    /// The workflow-relevant ones among them (at most 20).
    pub relevant: Vec<String>,
    /// More workflow-relevant files than listed.
    pub more: u32,
}

/// One page's bundle.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ExtContext {
    pub page: ExtPage,
    /// The folder of the page's repository (null: Not in Launchpad).
    pub folder: Option<ExtFolder>,
    /// Folders of the same repository in other workspaces.
    pub others: Vec<ExtFolder>,
    /// The workspace this answer is narrowed to: the folder's, else the one asked for. The extension follows it.
    pub workspace: Option<ExtWorkspace>,
    pub agents: Vec<ExtAgent>,
    pub suggestions: Vec<ExtSuggestion>,
    /// The private timeline of the workspace about this item (`launchpad note`, the owner's `note.write`), oldest first.
    pub notes: Vec<ExtNote>,
    pub local_prs: Vec<ExtLocalPr>,
    pub branches: Vec<ExtBranch>,
    /// Today item IDs about this page (its item, its agents, its Local PRs; on repository pages, the repository).
    pub today: Vec<String>,
    pub inbox: Option<ExtInbox>,
    /// Unread Inbox threads of the repository.
    pub inbox_unread: u32,
    pub defaults: ExtDefaults,
    /// The GitHub login Home is signed in with (the owner's): comments that mention it are highlighted.
    pub login: Option<String>,
    /// Presets for messages to bots.
    pub presets: Vec<ExtPreset>,
    /// A pull request page: its files that matter for CI, when Home has read them for its current head.
    pub ci: Option<ExtCi>,
}

/// The state of one row of a list page (pull requests, issues, notifications).
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ExtRow {
    /// The item as asked: a number, or a notification thread ID.
    pub item: String,
    pub repo: Option<String>,
    #[ts(type = "number | null")]
    pub number: Option<u64>,
    pub agents: Vec<ExtAgent>,
    pub suggestions: Vec<ExtSuggestion>,
    pub local_prs: Vec<ExtLocalPr>,
    pub today: Vec<String>,
    /// Its Inbox thread is unread (null: not in the Inbox).
    pub unread: Option<bool>,
    /// Its Inbox thread (Notifications: the thread of the row's item).
    pub thread: Option<String>,
    /// The folder of its repository in this workspace (null: Not in Launchpad).
    pub folder: Option<String>,
    /// The newest report about it.
    pub report: Option<ExtReport>,
    /// A bot already merged, closed, approved or answered it (its report is newer than the thread's activity): Done
    /// all handled marks it Done.
    pub handled: bool,
    /// Its pull request's Launchpad branch has commits GitHub does not have yet.
    pub push: Option<ExtBranch>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ExtRows {
    pub page: ExtPage,
    pub folder: Option<ExtFolder>,
    pub workspace: Option<ExtWorkspace>,
    pub rows: Vec<ExtRow>,
}

/// What one request reads once.
struct Data {
    projects: Vec<Project>,
    workspaces: Vec<Workspace>,
    sessions: Vec<Session>,
    /// Archived ones too: a suggestion's author may be archived.
    all_sessions: Vec<Session>,
    suggestions: Vec<Suggestion>,
    predrafts: Vec<Predraft>,
    today: Arc<Today>,
    login: Option<String>,
    reports: Vec<Report>,
}

/// A report card about a GitHub item, read straight from state.db (proposals live in the automation crate).
struct Report {
    id: String,
    workspace_id: Option<String>,
    session_id: Option<String>,
    /// `owner/repo#number`, lowercase.
    github: String,
    outcome: String,
    title: String,
    state: String,
    at: i64,
}

async fn reports(app: &App) -> Result<Vec<Report>> {
    let rows: Vec<String> = app
        .state_db
        .call(|c| {
            let mut q = c.prepare("SELECT json FROM launch_proposal WHERE json_extract(json, '$.body.kind') = 'report' AND json_extract(json, '$.body.github') IS NOT NULL AND json_extract(json, '$.state') != 'replaced'")?;
            Ok(q.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?)
        })
        .await?;
    let mut out: Vec<Report> = rows
        .iter()
        .filter_map(|j| serde_json::from_str::<serde_json::Value>(j).ok())
        .filter_map(|v| {
            Some(Report {
                id: v["id"].as_str()?.to_string(),
                workspace_id: v["workspace_id"].as_str().map(str::to_string),
                session_id: v["session_id"].as_str().map(str::to_string),
                github: v["body"]["github"].as_str()?.to_lowercase(),
                outcome: v["body"]["outcome"].as_str()?.to_string(),
                title: v["title"].as_str().unwrap_or("").to_string(),
                state: v["state"].as_str().unwrap_or("").to_string(),
                at: v["created_at"].as_i64().unwrap_or(0),
            })
        })
        .collect();
    out.sort_by_key(|r| std::cmp::Reverse(r.at));
    Ok(out)
}

/// Report outcomes that mean a bot already dealt with the item.
const HANDLED: &[&str] = &["merged", "closed", "approved", "commented"];

impl Data {
    async fn load(app: &App) -> Result<Data> {
        let login = crate::items::cached(app, "/user".into()).await.ok().flatten().and_then(|v| v["login"].as_str().map(str::to_string));
        Ok(Data {
            projects: launchpad_core::projects::list(app).await?,
            workspaces: launchpad_core::workspace::list(app).await?,
            sessions: app.sessions.list().into_iter().filter(|s| s.status != SessionStatus::Closed).collect(),
            all_sessions: app.sessions.list(),
            suggestions: crate::suggestions::list(app).await?.into_iter().filter(|s| s.status != "posted" && s.status != "posted_by_hand").collect(),
            predrafts: launchpad_core::predrafts::list(app).await?,
            today: launchpad_core::today::latest(app).await?,
            login,
            reports: reports(app).await?,
        })
    }
    /// The newest report of workspace `ws` about `repo#number`.
    fn report(&self, repo: &str, number: u64, ws: Option<&str>) -> Option<ExtReport> {
        let key = format!("{}#{number}", repo.to_lowercase());
        let r = self.reports.iter().find(|r| r.github == key && ws.is_some() && r.workspace_id.as_deref() == ws)?;
        let bot = r.session_id.as_deref().and_then(|id| self.all_sessions.iter().find(|s| s.id == id)).and_then(|s| s.launch_spec.bot.as_ref().map(|b| b.name.clone()));
        Some(ExtReport { id: r.id.clone(), outcome: r.outcome.clone(), bot, title: r.title.clone(), at: r.at, waiting: r.state == "proposed" })
    }
    fn workspace(&self, id: &str) -> Option<ExtWorkspace> {
        self.workspaces.iter().find(|w| w.id == id).map(|w| ExtWorkspace { id: w.id.clone(), name: w.name.clone() })
    }
    fn session_workspace<'a>(&'a self, s: &'a Session) -> Option<&'a str> {
        match s.project_id.as_deref().and_then(|p| self.projects.iter().find(|x| x.id == p)) {
            Some(p) => p.workspace_id.as_deref(),
            None => s.launch_spec.workspace_id.as_deref(),
        }
    }
}

fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// The folders of a repository with their relation; the best first (origin, upstream, fork; then the asked
/// workspace; then the oldest).
async fn folders(app: &App, d: &Data, repo: &str, prefer: Option<&str>) -> Vec<ExtFolder> {
    // A fork Home knows: its cached repository record names its parent, or a Local PR was published from it.
    let record = crate::items::cached(app, format!("/repos/{repo}")).await.ok().flatten();
    let parent = record.as_ref().and_then(|v| v["parent"]["full_name"].as_str().map(str::to_string));
    // The owner's repository of the same name counts as a fork only when the cache does not say it is none.
    let not_fork = record.as_ref().is_some_and(|v| v["fork"] == false);
    let owner = repo.split('/').next().unwrap_or("");
    let name = repo.split('/').nth(1).unwrap_or("");
    let yours = d.login.as_deref().is_some_and(|l| same(l, owner));
    let mut out: Vec<(Relation, i64, ExtFolder)> = vec![];
    for p in d.projects.iter().filter(|p| !p.is_temp) {
        let Some(ws) = p.workspace_id.as_deref() else { continue };
        let origin = p.gh_repo.as_deref();
        let relation = if origin.is_some_and(|o| same(o, repo)) {
            Relation::Origin
        } else if p.gh_upstream.as_deref().is_some_and(|u| same(u, repo)) {
            Relation::Upstream
        } else if origin.is_some_and(|o| {
            parent.as_deref().is_some_and(|x| same(x, o))
                || d.predrafts.iter().any(|x| x.project_id == p.id && x.push_repo.as_deref().is_some_and(|f| same(f, repo)))
                || (yours && !not_fork && !same(o, repo) && o.split('/').nth(1).is_some_and(|n| same(n, name)))
        }) {
            Relation::Fork
        } else {
            continue;
        };
        let folder = ExtFolder {
            id: p.id.clone(),
            name: p.name.clone(),
            workspace_id: ws.to_string(),
            workspace: d.workspace(ws).map(|w| w.name).unwrap_or_default(),
            relation,
            repo: p.gh_repo.clone(),
            upstream: p.gh_upstream.clone(),
            yours,
        };
        out.push((relation, p.created_at, folder));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| (Some(b.2.workspace_id.as_str()) == prefer).cmp(&(Some(a.2.workspace_id.as_str()) == prefer))).then(a.1.cmp(&b.1)));
    out.into_iter().map(|(_, _, f)| f).collect()
}

/// The page's item from Home's cache: title, head commit and head branch.
async fn cached_item(app: &App, page: &mut ExtPage) {
    let (Some(repo), Some(n)) = (&page.repo, page.number) else { return };
    let Ok(Some(v)) = crate::items::cached(app, format!("item:{repo}:{n}")).await else { return };
    page.title = v["title"].as_str().map(str::to_string);
    page.head = v["headRefOid"].as_str().map(str::to_string);
    page.head_branch = v["headRefName"].as_str().map(str::to_string);
    page.head_repo = v["headRepository"]["nameWithOwner"].as_str().map(str::to_string);
    page.maintainer_can_modify = v["maintainerCanModify"].as_bool();
}

/// What Launchpad has about one item (`number`) or a whole repository (None) in one workspace.
struct State {
    agents: Vec<ExtAgent>,
    suggestions: Vec<ExtSuggestion>,
    local_prs: Vec<ExtLocalPr>,
    today: Vec<String>,
}

fn local_pr(d: &Data, p: &Predraft) -> ExtLocalPr {
    let state = if p.archived { "discarded" } else if p.merged.is_some() { "merged" } else if p.published_url.is_some() { "published" } else { "open" };
    let session = p.session_id.as_deref().and_then(|id| d.all_sessions.iter().find(|s| s.id == id));
    let author = session.map(|s| s.launch_spec.bot.as_ref().map(|b| b.name.clone()).unwrap_or_else(|| s.title.clone()));
    let set = p.change_set.as_ref().map(|c| d.predrafts.iter().filter(|o| o.id != p.id && o.change_set.as_ref() == Some(c) && o.is_open()).count() as u32).unwrap_or(0);
    ExtLocalPr {
        id: p.id.clone(),
        title: p.title.clone(),
        branch: p.branch.clone(),
        base_branch: p.base_branch.clone(),
        session_id: p.session_id.clone(),
        state: state.into(),
        published_number: p.published_number,
        published_url: p.published_url.clone(),
        author,
        notes: p.notes.len() as u32,
        set,
        pushed_head: p.pushed_head.clone(),
    }
}

struct Item<'a> {
    repo: &'a str,
    number: Option<u64>,
    folder: Option<&'a ExtFolder>,
    workspace: Option<&'a str>,
    head: Option<&'a str>,
    head_branch: Option<&'a str>,
}

fn state(d: &Data, uses: &[Use], it: &Item) -> State {
    // Newest first: the first record of a session is its last use (and the comment it was last asked about).
    let mut named: HashMap<&str, (u64, Option<&str>)> = HashMap::new();
    for u in uses.iter().filter(|u| it.number.is_none_or(|x| x == u.number)) {
        named.entry(u.session_id.as_str()).or_insert((u.number, u.comment.as_deref()));
    }
    let mut agents: Vec<ExtAgent> = d
        .sessions
        .iter()
        .filter(|s| it.workspace.is_some() && d.session_workspace(s) == it.workspace)
        .filter_map(|s| {
            let by_ref = s.launch_spec.github_refs.iter().find(|r| same(&r.repo, it.repo) && it.number.is_none_or(|n| r.number == Some(n))).map(|r| r.number.unwrap_or(0));
            let number = by_ref.or_else(|| named.get(s.id.as_str()).map(|u| u.0));
            let comment = named.get(s.id.as_str()).and_then(|u| u.1).map(str::to_string);
            let in_folder = it.number.is_none() && it.folder.is_some_and(|f| s.project_id.as_deref() == Some(f.id.as_str()));
            if number.is_none() && !in_folder {
                return None;
            }
            Some(ExtAgent {
                session_id: s.id.clone(),
                title: s.title.clone(),
                status: s.status,
                needs_you: s.needs_you(),
                open_question: s.open_question,
                bot_id: s.launch_spec.bot.as_ref().map(|b| b.id.clone()),
                bot: s.launch_spec.bot.as_ref().map(|b| b.name.clone()),
                thread: s.launch_spec.bot.as_ref().map(|b| b.thread_id().to_string()),
                number: number.filter(|n| *n > 0),
                since: s.status_since,
                comment,
                last_message: s.last_message.clone(),
            })
        })
        .collect();
    agents.sort_by_key(|a| std::cmp::Reverse(a.since));
    agents.truncate(20);
    let suggestions: Vec<ExtSuggestion> = d
        .suggestions
        .iter()
        .filter_map(|s| {
            let SuggestionTarget::GitHub { repo, number, reply_to } = &s.target else { return None };
            if !same(repo, it.repo) || it.number.is_some_and(|n| n != *number) {
                return None;
            }
            let comments = s.comments.len() as u32;
            let outdated = match (&s.head, it.head) {
                (Some(h), Some(head)) if it.number.is_some() && h != head => comments,
                _ => 0,
            };
            let session = s.session_id.as_deref().and_then(|id| d.all_sessions.iter().find(|x| x.id == id));
            let bot = session.and_then(|x| x.launch_spec.bot.as_ref().map(|b| b.name.clone()));
            let attribution = session.map(launchpad_core::attribution::line).unwrap_or_else(|| launchpad_core::attribution::text(None, "Claude Code"));
            let text = format!("{}{}", reply_to.as_ref().map(ReplyTo::prefix).unwrap_or_default(), s.body);
            let lines = s
                .comments
                .iter()
                .map(|c| ExtLine { part: crate::ledger::line_part(c), path: c.path.clone(), line: c.line, start_line: c.start_line, side: c.side.clone(), body: c.body.clone(), state: None })
                .collect();
            let author = bot.clone().or_else(|| session.map(|x| x.title.clone())).unwrap_or_else(|| s.author.clone());
            Some(ExtSuggestion { id: s.id.clone(), kind: s.kind.clone(), number: *number, head: s.head.clone(), comments, outdated, status: s.status.clone(), session_id: s.session_id.clone(), bot, reply_to: reply_to.clone(), text, attribution, event: s.event.clone(), lines, created_at: s.created_at, author, body: s.body.clone() })
        })
        .collect();
    let local_prs: Vec<ExtLocalPr> = match it.folder {
        Some(f) => d
            .predrafts
            .iter()
            .filter(|p| p.project_id == f.id && !p.archived)
            .filter(|p| local_pr_matches(p, it.number, it.head_branch))
            .map(|p| local_pr(d, p))
            .collect(),
        None => vec![],
    };
    let sessions: HashSet<&str> = agents.iter().map(|a| a.session_id.as_str()).collect();
    let prs: HashSet<&str> = local_prs.iter().map(|p| p.id.as_str()).collect();
    let item_ref = it.number.map(|n| format!("{}#{n}", it.repo.to_lowercase()));
    let repo_prefix = format!("{}#", it.repo.to_lowercase());
    let today = d
        .today
        .items
        .iter()
        .filter(|i| i.workspaces.is_empty() || it.workspace.is_some_and(|w| i.workspaces.iter().any(|x| x == w)))
        .filter(|i| {
            let gh = i.github.as_deref().map(str::to_lowercase);
            let about = match &item_ref {
                Some(r) => gh.as_deref() == Some(r.as_str()),
                None => gh.as_deref().is_some_and(|g| g.starts_with(&repo_prefix)),
            };
            about || i.session_id.as_deref().is_some_and(|s| sessions.contains(s)) || prs.contains(i.record.as_str())
        })
        .map(|i| i.id.clone())
        .collect();
    State { agents, suggestions, local_prs, today }
}

/// The followed branches of the folder for this item (its pull request) or all of them, with whether GitHub has
/// Home's commit.
async fn branches(app: &App, folder: Option<&ExtFolder>, number: Option<u64>) -> Vec<ExtBranch> {
    let Some(f) = folder else { return vec![] };
    if !launchpad_core::home_git::here(app) {
        return vec![];
    }
    let follows = launchpad_core::home_git::follows(app, &f.id).await.unwrap_or_default();
    let mut out = vec![];
    for (branch, follow) in follows.into_iter().take(20) {
        if let Some(n) = number {
            // The folder follows pull requests of its own repository (or its upstream): the number names it.
            if !follow.url.trim_end_matches('/').ends_with(&format!("/pull/{n}")) {
                continue;
            }
        }
        let head = launchpad_core::home_git::tip(app, &f.id, &format!("refs/heads/{branch}")).await.ok().flatten();
        let not_on_github = head.is_some() && head != follow.forwarded;
        out.push(ExtBranch { branch, url: follow.url, head, not_on_github, error: follow.error, push_ready: None });
    }
    out
}

/// The agents' Launchpad branches of the folder that hold the pull request's head `pr_head` and more commits, and
/// that Home does not push to GitHub yet (not followed): what Push to this PR would send.
async fn push_ready(app: &App, f: &ExtFolder, agents: &[ExtAgent], followed: &[ExtBranch], number: u64, pr_head: &str, url: &str) -> Vec<ExtBranch> {
    if !launchpad_core::home_git::here(app) {
        return vec![];
    }
    let mut seen: HashSet<String> = followed.iter().map(|b| b.branch.clone()).collect();
    let mut out = vec![];
    for a in agents.iter().filter(|a| a.number == Some(number)).take(10) {
        let records = launchpad_core::away::session_records(app, &a.session_id).await.unwrap_or_default();
        for w in records.into_iter().filter(|w| w.project_id == f.id) {
            if !seen.insert(w.branch.clone()) {
                continue;
            }
            let Ok(Some(tip)) = launchpad_core::home_git::tip(app, &f.id, &format!("refs/heads/{}", w.branch)).await else { continue };
            if tip == pr_head {
                continue;
            }
            let Some(ahead) = launchpad_core::home_git::ahead(app, &f.id, pr_head, &tip).await.filter(|n| *n > 0) else { continue };
            let push_ready = Some(ExtPushReady { branch: w.branch.clone(), ahead, head: tip.clone(), session_id: Some(a.session_id.clone()), title: Some(a.title.clone()) });
            out.push(ExtBranch { branch: w.branch, url: url.to_string(), head: Some(tip), not_on_github: false, error: None, push_ready });
        }
    }
    out
}

/// Whether a changed file decides what CI runs: workflows and actions, scripts, build and dependency files.
fn ci_relevant(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    lower.starts_with(".github/")
        || lower.starts_with("scripts/")
        || lower.starts_with(".circleci/")
        || lower.contains("/scripts/")
        || [".sh", ".bash", ".ps1", ".bat", ".cmd"].iter().any(|e| name.ends_with(e))
        || name.starts_with("dockerfile")
        || name.starts_with("requirements")
        || [
            "makefile", "justfile", "package.json", "package-lock.json", "pnpm-lock.yaml", "yarn.lock", "pyproject.toml", "setup.py", "setup.cfg", "tox.ini", "noxfile.py",
            "conftest.py", "uv.lock", "poetry.lock", "pipfile", "pipfile.lock", "cargo.toml", "cargo.lock", "build.rs", "go.mod", "go.sum", "gemfile", "gemfile.lock",
            ".pre-commit-config.yaml", "docker-compose.yml", "docker-compose.yaml", ".gitlab-ci.yml", ".travis.yml", "action.yml", "action.yaml",
        ]
        .contains(&name)
}

/// The pull request's files that matter for CI, from Home's cached diff of its current head (never a GitHub call).
async fn ci(app: &App, repo: &str, number: Option<u64>) -> Option<ExtCi> {
    let n = number?;
    let item = crate::items::cached(app, format!("item:{repo}:{n}")).await.ok().flatten()?;
    let (base, head) = (&item["baseRefOid"], item["headRefOid"].as_str()?);
    let project = launchpad_core::projects::for_repo(app, repo).await.ok().flatten().map(|p| p.id).unwrap_or_else(|| "api".into());
    let mut diff = None;
    for key in [project.as_str(), "api"] {
        diff = crate::items::cached(app, format!("diff:{repo}:{base}:{}:{key}", serde_json::json!(head))).await.ok().flatten();
        if diff.is_some() {
            break;
        }
    }
    let files: Vec<String> = diff?["files"].as_array()?.iter().filter_map(|f| f["filename"].as_str().map(str::to_string)).collect();
    let relevant: Vec<String> = files.iter().filter(|f| ci_relevant(f)).cloned().collect();
    Some(ExtCi { head: head.to_string(), files: files.len() as u32, more: relevant.len().saturating_sub(20) as u32, relevant: relevant.into_iter().take(20).collect() })
}

/// The owner's last step with each line comment of the page's suggested reviews (the feedback ledger).
async fn ledger_states(app: &App, suggestions: &mut [ExtSuggestion]) {
    for s in suggestions.iter_mut().filter(|s| !s.lines.is_empty()) {
        let records = crate::ledger::list(app, Some(&s.id)).await.unwrap_or_default();
        for l in &mut s.lines {
            let Some(r) = records.iter().find(|r| r.part == l.part) else { continue };
            l.state = r.events.iter().rev().map(|e| e.event.as_str()).find(|e| matches!(*e, "inserted" | "posted_by_hand" | "confirmed")).map(str::to_string);
        }
    }
}

/// `DiffSelection::lines`: the selected lines of a pull request's diff (Home's cached diff of its head, else read
/// from GitHub), numbered, with `+`/`-`/space marks, and a first line naming the head when it moved since the owner
/// selected them. None when the file or lines are not in the diff.
pub async fn selection_lines(app: &App, sel: &DiffSelection) -> Option<String> {
    let v = match crate::items::diff(app, &sel.repo, sel.number).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("selection of {}#{}: {e:#}", sel.repo, sel.number);
            return None;
        }
    };
    let file = v["files"].as_array()?.iter().find(|f| f["filename"].as_str() == Some(sel.path.as_str()))?;
    let picked = pick_lines(file["patch"].as_str()?, sel.side == "LEFT", sel.start, sel.end);
    if picked.is_empty() {
        return None;
    }
    let head = v["head"].as_str().unwrap_or_default();
    let mut out = String::new();
    if let Some(h) = sel.head.as_deref().filter(|h| !head.is_empty() && !head.starts_with(*h)) {
        out.push_str(&format!("@@ read at {} (the owner selected the lines at {}: the numbers may have moved) @@\n", &head[..head.len().min(12)], &h[..h.len().min(12)]));
    }
    out.push_str(&picked.join("\n"));
    Some(out)
}

/// The lines of a unified diff patch whose old (`left`) or new line number is in `start..=end`, numbered and marked.
fn pick_lines(patch: &str, left: bool, start: u64, end: u64) -> Vec<String> {
    let (mut old, mut new) = (0u64, 0u64);
    let mut out = vec![];
    let cut = |t: &str| t.chars().take(400).collect::<String>();
    for line in patch.lines() {
        if let Some(h) = line.strip_prefix("@@ ") {
            // `-a,b +c,d @@ <the enclosing function>`: only the ranges, never the function's text.
            for part in h.split(" @@").next().unwrap_or_default().split_whitespace() {
                let num = |p: &str| p.split(',').next().and_then(|n| n.parse::<u64>().ok());
                if let Some(n) = part.strip_prefix('-').and_then(num) {
                    old = n;
                } else if let Some(n) = part.strip_prefix('+').and_then(num) {
                    new = n;
                }
            }
            continue;
        }
        let (mark, text) = line.split_at(line.chars().next().map(char::len_utf8).unwrap_or(0));
        let at = match mark {
            "+" => {
                new += 1;
                (!left).then_some(new - 1)
            }
            "-" => {
                old += 1;
                left.then_some(old - 1)
            }
            " " | "" => {
                old += 1;
                new += 1;
                Some(if left { old - 1 } else { new - 1 })
            }
            _ => None,
        };
        if let Some(n) = at.filter(|n| (start..=end).contains(n)) {
            out.push(format!("{n:>5} {}{}", if mark.is_empty() { " " } else { mark }, cut(text)));
            if out.len() >= 400 {
                break;
            }
        }
    }
    out
}

/// `branch.push-to-pr`: the owner's Push to this PR. Reads the pull request on GitHub, then pushes the Launchpad
/// branch at `head` (the commit the owner saw) to the pull request's branch: its repository, or the fork when the
/// author lets maintainers edit (or the fork is the owner's), only as a fast-forward of its head, never forced
/// (`home_git::push_to_pull_request`). Home then follows the branch (`git_follow`), so the agent's next commits go
/// there too, like a session launched with Push to PR.
pub async fn push_to_pr(app: &App, repo: &str, number: u64, branch: &str, head: &str) -> Result<serde_json::Value> {
    use anyhow::{Context, ensure};
    crate::items::validate(repo)?;
    ensure!(branch.starts_with("lp/") && branch.len() <= 255 && !branch.contains(".."), "only Launchpad branches (lp/…) go to a pull request");
    ensure!(head.len() == 40 && head.bytes().all(|b| b.is_ascii_hexdigit()), "give the branch's commit you saw (40 characters)");
    let label = format!("{repo}#{number}");
    let v = app.github.get(&format!("/repos/{repo}/pulls/{number}")).await.with_context(|| format!("could not read pull request {label}"))?;
    ensure!(v["state"] == "open", "pull request {label} is not open");
    let head_repo = v["head"]["repo"]["full_name"].as_str().with_context(|| format!("the branch repository of {label} was deleted"))?.to_string();
    let target = v["head"]["ref"].as_str().context("pull request branch missing")?.to_string();
    let pr_head = v["head"]["sha"].as_str().context("pull request head missing")?.to_string();
    let url = v["html_url"].as_str().context("pull request URL missing")?.to_string();
    let projects = launchpad_core::projects::list(app).await?;
    let mut project = None;
    for p in projects.iter().filter(|p| p.home_git && !p.is_temp) {
        let names = [p.gh_repo.as_deref(), p.gh_upstream.as_deref()];
        if names.iter().flatten().any(|r| same(r, repo) || same(r, &head_repo)) && launchpad_core::home_git::tip(app, &p.id, &format!("refs/heads/{branch}")).await?.is_some() {
            project = Some(p.clone());
            break;
        }
    }
    let p = project.with_context(|| format!("no folder of {repo} has the branch {branch}"))?;
    if let Some(f) = launchpad_core::home_git::follow(app, &p.id, branch).await? {
        ensure!(f.url.trim_end_matches('/') == url.trim_end_matches('/'), "{branch} already goes to {}", f.url);
    }
    let own = p.gh_repo.as_deref().is_some_and(|r| same(r, &head_repo));
    let login = crate::items::cached(app, "/user".into()).await.ok().flatten().and_then(|u| u["login"].as_str().map(str::to_string));
    let yours = login.is_some_and(|l| head_repo.split('/').next().is_some_and(|o| same(o, &l)));
    ensure!(own || yours || v["maintainer_can_modify"] == true, "the author of {label} does not let maintainers edit its branch: Home cannot push there");
    let remote = if own {
        "origin".to_string()
    } else {
        launchpad_core::predrafts::fork_url(&p.remote_url.clone().unwrap_or_default(), &p.gh_repo.clone().unwrap_or_default(), &head_repo).with_context(|| format!("cannot derive the URL of {head_repo}"))?
    };
    launchpad_core::home_git::push_to_pull_request(app, &p, branch, head, &remote, &target, &pr_head).await?;
    // The session whose branch it is (`lp/<slug>-<session8>`) hears of it and of later pushes.
    let short = branch.rsplit('-').next().unwrap_or_default();
    let mut session = None;
    for s in app.sessions.list().into_iter().filter(|s| short.len() == 8 && s.id.starts_with(short)) {
        if launchpad_core::away::session_branch(app, &s.id, &p.id, branch).await.ok().flatten().is_some() {
            session = Some(s.id);
            break;
        }
    }
    let follow = launchpad_core::home_git::Follow { remote, branch: target.clone(), url: url.clone(), session_id: session.clone(), forwarded: Some(head.to_string()), error: None };
    launchpad_core::home_git::set_follow(app, &p.id, branch, &follow).await?;
    app.bus.publish("home_git", "followed", serde_json::json!({"project_id": p.id, "branch": branch, "url": url}));
    if let Some(sid) = &session {
        let text = format!("The owner pushed {branch} ({}) to {url} ({target}). Home now pushes your next commits on this branch there too; it never force-pushes.", &head[..12]);
        launchpad_core::sessions::notice_anywhere(app, sid, &text).await;
    }
    Ok(serde_json::json!({"ok": true, "url": url, "branch": target, "head": head}))
}

/// The default bot: the folder's setting, else the last bot used on the repository, else the workspace's Ask bot.
pub(crate) async fn default_bot(app: &App, project: Option<&Project>, repo: Option<&str>, workspace: Option<&str>) -> Option<ExtBot> {
    let ws = workspace?;
    let bots = launchpad_core::bots::briefs(app, ws).await.ok()?;
    let pick = |b: &launchpad_core::bots::Brief, source: &str| ExtBot { id: b.id.clone(), name: b.name.clone(), slug: b.slug.clone(), source: source.into() };
    if let Some(b) = project.and_then(|p| p.bot.as_deref()).and_then(|id| bots.iter().find(|b| b.id == id)) {
        return Some(pick(b, "folder"));
    }
    if let Some(repo) = repo {
        for id in launchpad_core::ref_use::bots(app, repo).await.unwrap_or_default() {
            if let Some(b) = bots.iter().find(|b| b.id == id) {
                return Some(pick(b, "repository"));
            }
        }
    }
    launchpad_core::bots::ask_target(&bots).map(|b| pick(b, "ask"))
}

/// The bundle of a github.com page. `workspace`: the extension's current workspace, used when the repository has no
/// folder and to choose among folders of several workspaces.
pub async fn context(app: &App, url: &str, workspace: Option<&str>) -> Result<ExtContext> {
    let d = Data::load(app).await?;
    let mut page = parse_page(url);
    let asked = workspace.filter(|w| d.workspaces.iter().any(|x| x.id == *w));
    let mut found = match &page.repo {
        Some(r) => folders(app, &d, r, asked).await,
        None => vec![],
    };
    let folder = (!found.is_empty()).then(|| found.remove(0));
    let others: Vec<ExtFolder> = found.into_iter().filter(|f| folder.as_ref().is_none_or(|x| x.workspace_id != f.workspace_id)).collect();
    let ws = folder.as_ref().map(|f| f.workspace_id.clone()).or(asked.map(str::to_string));
    cached_item(app, &mut page).await;
    let project = folder.as_ref().and_then(|f| d.projects.iter().find(|p| p.id == f.id));
    let mut out = ExtContext {
        workspace: ws.as_deref().and_then(|w| d.workspace(w)),
        folder: folder.clone(),
        others,
        agents: vec![],
        suggestions: vec![],
        notes: vec![],
        local_prs: vec![],
        branches: vec![],
        today: vec![],
        inbox: None,
        inbox_unread: 0,
        defaults: ExtDefaults { bot: default_bot(app, project, page.repo.as_deref(), ws.as_deref()).await, review: match page.repo.as_deref() { Some(r) => review_choice(app, r).await.unwrap_or(None), None => None } },
        login: d.login.clone(),
        presets: presets(app),
        ci: None,
        page: page.clone(),
    };
    let Some(repo) = page.repo.as_deref() else { return Ok(out) };
    let uses = launchpad_core::ref_use::uses(app, repo).await.unwrap_or_default();
    let item = Item { repo, number: page.number, folder: folder.as_ref(), workspace: ws.as_deref(), head: page.head.as_deref(), head_branch: page.head_branch.as_deref() };
    let s = state(&d, &uses, &item);
    (out.agents, out.suggestions, out.local_prs, out.today) = (s.agents, s.suggestions, s.local_prs, s.today);
    if let (Some(n), Some(w)) = (page.number, ws.as_deref()) {
        let notes: Vec<_> = crate::notes::of_item(app, repo, n).await.unwrap_or_default().into_iter().filter(|x| x.workspace_id.as_deref() == Some(w)).collect();
        let all = if notes.iter().any(|x| x.suggestion_id.is_some()) { crate::suggestions::list(app).await.unwrap_or_default() } else { vec![] };
        out.notes = notes
            .into_iter()
            .map(|x| {
                let suggestion = x.suggestion_id.map(|id| match all.iter().find(|s| s.id == id) {
                    Some(s) => ExtNoteSuggestion { id, status: s.status.clone(), url: s.url.clone() },
                    None => ExtNoteSuggestion { id, status: "discarded".into(), url: None },
                });
                ExtNote { id: x.id, author: x.author, bot_id: x.bot_id, session_id: x.session_id, by_owner: x.by_owner, body: x.body, created_at: x.created_at, head: x.head, reply_to: x.reply_to, suggestion }
            })
            .collect();
    }
    out.branches = branches(app, folder.as_ref(), page.number).await;
    if page.kind.is_pr() && let (Some(f), Some(n), Some(head)) = (folder.as_ref(), page.number, page.head.as_deref()) {
        let url = format!("https://github.com/{repo}/pull/{n}");
        out.branches.extend(push_ready(app, f, &out.agents, &out.branches, n, head, &url).await);
    }
    if page.kind.is_pr() {
        out.ci = ci(app, repo, page.number).await;
    }
    ledger_states(app, &mut out.suggestions).await;
    let threads = crate::inbox::of(app).threads_of(app, Some(repo), &[]).await.unwrap_or_default();
    out.inbox_unread = threads.iter().filter(|t| t.unread).count() as u32;
    if let Some(n) = page.number {
        out.inbox = threads.iter().find(|t| t.number == Some(n)).map(|t| ExtInbox { thread: t.id.clone(), unread: t.unread, reason: t.reason.clone() });
    }
    Ok(out)
}

/// The presets for messages to bots.
fn presets(app: &App) -> Vec<ExtPreset> {
    app.config.get().presets.iter().filter(|p| p.bots).map(|p| ExtPreset { id: p.id.clone(), label: p.label.clone(), text: p.text.clone() }).collect()
}

/// The repository's remembered Review with agent choice.
pub async fn review_choice(app: &App, repo: &str) -> Result<Option<ExtReviewChoice>> {
    let repo = repo.to_lowercase();
    let json: Option<String> = app
        .state_db
        .call(move |c| {
            use rusqlite::OptionalExtension;
            Ok(c.query_row("SELECT json FROM repo_choice WHERE repo = ?1 AND what = 'review'", [repo], |r| r.get(0)).optional()?)
        })
        .await?;
    Ok(json.and_then(|j| serde_json::from_str(&j).ok()))
}

/// `github.review-choice`: remembers the bot and preset of the repository's Review with agent (owner only). The bot
/// must be one of `workspace`'s, the preset one of the presets for bots.
pub async fn set_review_choice(app: &App, repo: &str, workspace: &str, choice: ExtReviewChoice) -> Result<ExtReviewChoice> {
    anyhow::ensure!(launchpad_core::github::valid_repo(repo), "not a repository name");
    if let Some(bot) = &choice.bot {
        let bots = launchpad_core::bots::briefs(app, workspace).await?;
        anyhow::ensure!(bots.iter().any(|b| &b.id == bot), "no such bot in this workspace");
    }
    if let Some(preset) = &choice.preset {
        anyhow::ensure!(presets(app).iter().any(|p| &p.id == preset), "no such preset for bots");
    }
    let (repo, json, at) = (repo.to_lowercase(), serde_json::to_string(&choice)?, launchpad_core::util::now_ms());
    app.state_db
        .call(move |c| {
            c.execute("INSERT INTO repo_choice (repo, what, json, at) VALUES (?1, 'review', ?2, ?3) ON CONFLICT (repo, what) DO UPDATE SET json = ?2, at = ?3", rusqlite::params![repo, json, at])?;
            Ok(())
        })
        .await?;
    Ok(choice)
}

/// One row asked for: its item as given, its repository and number.
struct Wanted {
    item: String,
    repo: String,
    number: Option<u64>,
}

/// `owner/repo#number`.
fn item_ref(s: &str) -> Option<(String, u64)> {
    let (repo, n) = s.split_once('#')?;
    let n = n.parse::<u64>().ok().filter(|n| *n > 0)?;
    launchpad_core::github::valid_repo(repo).then(|| (repo.to_string(), n))
}

/// The row states of a list page: `items` are numbers on a repository's Pull requests or Issues page, thread IDs or
/// `owner/repo#number` on Notifications (at most 100).
pub async fn rows(app: &App, url: &str, items: &[String], workspace: Option<&str>) -> Result<ExtRows> {
    let d = Data::load(app).await?;
    let page = parse_page(url);
    let asked = workspace.filter(|w| d.workspaces.iter().any(|x| x.id == *w));
    let items: Vec<&String> = items.iter().take(100).collect();
    let wanted: Vec<Wanted> = match (&page.repo, page.kind) {
        (_, PageKind::Notifications) => {
            let ids: Vec<String> = items.iter().filter(|s| !s.contains('#')).map(|s| s.to_string()).collect();
            let threads = if ids.is_empty() { vec![] } else { crate::inbox::of(app).threads_of(app, None, &ids).await.unwrap_or_default() };
            items
                .iter()
                .filter_map(|s| match item_ref(s) {
                    Some((repo, n)) => Some(Wanted { item: s.to_string(), repo, number: Some(n) }),
                    None => threads.iter().find(|t| &&t.id == s).map(|t| Wanted { item: s.to_string(), repo: t.repo.clone(), number: t.number }),
                })
                .collect()
        }
        (Some(repo), _) => items.iter().filter_map(|s| s.parse::<u64>().ok().map(|n| Wanted { item: s.to_string(), repo: repo.clone(), number: Some(n) })).collect(),
        (None, _) => vec![],
    };
    // Per repository: its folder, the items its sessions named, its Inbox threads and the branches Home pushes to its
    // pull requests.
    struct PerRepo {
        folder: Option<ExtFolder>,
        uses: Vec<Use>,
        threads: Vec<crate::inbox::Thread>,
        branches: Vec<ExtBranch>,
    }
    let mut by_repo: HashMap<String, PerRepo> = HashMap::new();
    let mut rows = vec![];
    for w in wanted {
        let key = w.repo.to_lowercase();
        if !by_repo.contains_key(&key) {
            let folder = folders(app, &d, &w.repo, asked).await.into_iter().next();
            let uses = launchpad_core::ref_use::uses(app, &w.repo).await.unwrap_or_default();
            let threads = crate::inbox::of(app).threads_of(app, Some(&w.repo), &[]).await.unwrap_or_default();
            let branches = branches(app, folder.as_ref(), None).await;
            by_repo.insert(key.clone(), PerRepo { folder, uses, threads, branches });
        }
        let r = &by_repo[&key];
        let ws = r.folder.as_ref().map(|f| f.workspace_id.as_str()).or(asked);
        // A notification about something else than an issue or pull request (a release, a discussion) has no state.
        let s = match w.number {
            Some(n) => state(&d, &r.uses, &Item { repo: &w.repo, number: Some(n), folder: r.folder.as_ref(), workspace: ws, head: None, head_branch: None }),
            None => State { agents: vec![], suggestions: vec![], local_prs: vec![], today: vec![] },
        };
        let thread = w.number.and_then(|n| r.threads.iter().filter(|t| t.number == Some(n)).max_by(|a, b| a.updated_at.cmp(&b.updated_at)));
        let report = w.number.and_then(|n| d.report(&w.repo, n, ws));
        let handled = handled(thread, report.as_ref());
        let push = w.number.and_then(|n| r.branches.iter().find(|b| b.not_on_github && b.url.trim_end_matches('/').ends_with(&format!("/pull/{n}")))).cloned();
        rows.push(ExtRow {
            item: w.item,
            repo: Some(w.repo.clone()),
            number: w.number,
            agents: s.agents,
            suggestions: s.suggestions,
            local_prs: s.local_prs,
            today: s.today,
            unread: thread.map(|t| t.unread),
            thread: thread.map(|t| t.id.clone()),
            folder: r.folder.as_ref().filter(|f| Some(f.workspace_id.as_str()) == ws).map(|f| f.id.clone()),
            report,
            handled,
            push,
        });
    }
    let folder = page.repo.as_ref().and_then(|r| by_repo.get(&r.to_lowercase())).and_then(|x| x.folder.clone());
    let folder = match (folder, &page.repo) {
        (Some(f), _) => Some(f),
        (None, Some(r)) => folders(app, &d, r, asked).await.into_iter().next(),
        (None, None) => None,
    };
    let ws = folder.as_ref().map(|f| f.workspace_id.clone()).or(asked.map(str::to_string));
    Ok(ExtRows { workspace: ws.as_deref().and_then(|w| d.workspace(w)), folder, page, rows })
}

/// A thread of the Inbox that a bot's report already dealt with: the report says merged, closed, approved or
/// commented, and nothing happened on the thread for more than an hour after it.
fn handled(thread: Option<&crate::inbox::Thread>, report: Option<&ExtReport>) -> bool {
    let (Some(t), Some(r)) = (thread, report) else { return false };
    HANDLED.contains(&r.outcome.as_str()) && launchpad_core::today::parse_time(&t.updated_at) <= r.at + 3_600_000
}

/// Today's counts of a workspace as the notifications strip shows them.
#[derive(Clone, Debug, Default, Serialize, Deserialize, TS)]
pub struct ExtTodayCounts {
    /// Agents' questions and reports that ask you.
    pub answer: u32,
    /// Finished, failed or interrupted agents not seen yet.
    pub review: u32,
    /// Agent-written comments and reviews to post.
    pub suggestions: u32,
    /// Local PRs to publish.
    pub local_prs: u32,
    /// Everything that waits in Today.
    pub total: u32,
}

/// An Inbox thread a bot already handled (Done all handled).
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ExtHandled {
    pub thread: String,
    pub repo: String,
    #[ts(type = "number")]
    pub number: u64,
    pub title: String,
    pub outcome: String,
    pub bot: Option<String>,
}

/// The notifications page's strip (N1) and the threads of Done all handled (N4).
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ExtNotifications {
    pub workspace: Option<ExtWorkspace>,
    pub counts: ExtTodayCounts,
    pub handled: Vec<ExtHandled>,
    /// Threads marked Done, read or unsubscribed on github.com in the last day (recorded in Launchpad's Inbox).
    pub observed: u32,
}

fn counts(today: &Today, ws: Option<&str>) -> ExtTodayCounts {
    use launchpad_core::today::{Filter, Kind};
    let mut c = ExtTodayCounts::default();
    for i in today.items.iter().filter(|i| i.workspaces.is_empty() || ws.is_some_and(|w| i.workspaces.iter().any(|x| x == w))) {
        c.total += 1;
        if i.filter == Filter::Answer {
            c.answer += 1;
        }
        match i.kind {
            Kind::Finished | Kind::Failed => c.review += 1,
            Kind::Suggestion => c.suggestions += 1,
            Kind::LocalPr => c.local_prs += 1,
            _ => {}
        }
    }
    c
}

/// The strip of github.com/notifications for workspace `ws`: Today's counts and the Inbox threads of the workspace's
/// repositories that a bot already handled.
pub async fn notifications(app: &App, ws: Option<&str>) -> Result<ExtNotifications> {
    let d = Data::load(app).await?;
    let ws = ws.filter(|w| d.workspaces.iter().any(|x| x.id == *w));
    let snapshot = crate::inbox::of(app).snapshot(app).await?;
    let mut handled_threads = vec![];
    if let Some(w) = ws {
        let repos: HashSet<String> = launchpad_core::scope::repos(app, w).await.into_iter().collect();
        for t in snapshot.threads.iter().filter(|t| repos.contains(&t.repo.to_lowercase())) {
            let Some(n) = t.number else { continue };
            let Some(r) = d.report(&t.repo, n, Some(w)) else { continue };
            if handled(Some(t), Some(&r)) {
                handled_threads.push(ExtHandled { thread: t.id.clone(), repo: t.repo.clone(), number: n, title: t.title.clone(), outcome: r.outcome, bot: r.bot });
            }
        }
    }
    Ok(ExtNotifications { workspace: ws.and_then(|w| d.workspace(w)), counts: counts(&d.today, ws), handled: handled_threads, observed: snapshot.observed.len() as u32 })
}

fn local_pr_matches(p: &Predraft, number: Option<u64>, head_branch: Option<&str>) -> bool {
    match number {
        Some(n) => launchpad_core::predrafts::fixed_issues(&p.body).contains(&n)
            || p.published_number == Some(n)
            || head_branch.is_some_and(|b| p.remote_branch.as_deref() == Some(b) || p.branch == b),
        None => p.is_open(),
    }
}
#[cfg(test)]
mod item_link_tests {
    #[test]
    fn unpublished_local_pr_matches_fixed_issue_and_pr_branch() {
        let mut p: super::Predraft = serde_json::from_value(serde_json::json!({
            "id":"p", "project_id":"f", "branch":"lp/fix", "base_branch":"main",
            "initial_base":"abc", "title":"Fix", "body":"Fixes #12. Closes #34",
            "notes":[], "archived":false, "default_remote_branch":"fix", "created_at":0
        })).unwrap();
        assert!(super::local_pr_matches(&p, Some(12), None));
        assert!(super::local_pr_matches(&p, Some(34), None));
        assert!(!super::local_pr_matches(&p, Some(56), None));
        assert!(super::local_pr_matches(&p, Some(56), Some("lp/fix")));
        p.body.clear();
        assert!(!super::local_pr_matches(&p, Some(12), None));
    }
}

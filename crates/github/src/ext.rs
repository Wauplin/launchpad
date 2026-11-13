//! The Chrome extension's page context (`GET /api/ext/context`) and row states (`GET /api/ext/rows`): what Launchpad
//! knows about a github.com page, answered from Home's records and cache.db only (never a GitHub API call on the
//! request path), in one call per navigation. The extension's device is the owner, but each answer is narrowed to one
//! workspace: the folder's for a repository in Launchpad, else the one the extension shows.
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Result;
use launchpad_core::App;
use launchpad_core::model::{Project, Session, SessionStatus};
use launchpad_core::predrafts::Predraft;
use launchpad_core::today::Today;
use launchpad_core::workspace::Workspace;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::suggestions::{Suggestion, SuggestionKind, SuggestionTarget};

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
}

/// GitHub's top-level paths that are not owners.
const RESERVED: &[&str] = &[
    "about", "account", "apps", "codespaces", "collections", "contact", "customer-stories", "dashboard", "enterprise", "enterprises", "explore", "features",
    "issues", "login", "logout", "marketplace", "new", "notifications", "organizations", "orgs", "pricing", "pulls", "readme", "search", "security",
    "sessions", "settings", "signup", "site", "sponsors", "stars", "team", "topics", "trending", "users", "watching",
];

/// The page of a github.com URL. Anything else (another host, a malformed path) is `Other` without a repository.
pub fn parse_page(url: &str) -> ExtPage {
    let mut page = ExtPage { kind: PageKind::Other, repo: None, number: None, run: None, title: None, head: None, head_branch: None };
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
    /// The issue or pull request it is about (null: the repository or its folder).
    #[ts(type = "number | null")]
    pub number: Option<u64>,
    #[ts(type = "number")]
    pub since: i64,
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
    /// `draft`, `posting` or `uncertain`.
    pub status: String,
    pub session_id: Option<String>,
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
}

/// Commits of a Launchpad branch that could go to an existing pull request (filled from M3 on).
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ExtPushReady {
    pub branch: String,
    pub ahead: u32,
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
    /// Always null until M3.
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
    pub local_prs: Vec<ExtLocalPr>,
    pub branches: Vec<ExtBranch>,
    /// Today item IDs about this page (its item, its agents, its Local PRs; on repository pages, the repository).
    pub today: Vec<String>,
    pub inbox: Option<ExtInbox>,
    /// Unread Inbox threads of the repository.
    pub inbox_unread: u32,
    pub defaults: ExtDefaults,
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
    suggestions: Vec<Suggestion>,
    predrafts: Vec<Predraft>,
    today: Arc<Today>,
    login: Option<String>,
}

impl Data {
    async fn load(app: &App) -> Result<Data> {
        let login = crate::items::cached(app, "/user".into()).await.ok().flatten().and_then(|v| v["login"].as_str().map(str::to_string));
        Ok(Data {
            projects: launchpad_core::projects::list(app).await?,
            workspaces: launchpad_core::workspace::list(app).await?,
            sessions: app.sessions.list().into_iter().filter(|s| s.status != SessionStatus::Closed).collect(),
            suggestions: crate::suggestions::list(app).await?.into_iter().filter(|s| s.status != "posted").collect(),
            predrafts: launchpad_core::predrafts::list(app).await?,
            today: launchpad_core::today::latest(app).await?,
            login,
        })
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
    let parent = crate::items::cached(app, format!("/repos/{repo}")).await.ok().flatten().and_then(|v| v["parent"]["full_name"].as_str().map(str::to_string));
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
                || (yours && !same(o, repo) && o.split('/').nth(1).is_some_and(|n| same(n, name)))
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
}

/// What Launchpad has about one item (`number`) or a whole repository (None) in one workspace.
struct State {
    agents: Vec<ExtAgent>,
    suggestions: Vec<ExtSuggestion>,
    local_prs: Vec<ExtLocalPr>,
    today: Vec<String>,
}

fn local_pr(p: &Predraft) -> ExtLocalPr {
    let state = if p.archived { "discarded" } else if p.merged.is_some() { "merged" } else if p.published_url.is_some() { "published" } else { "open" };
    ExtLocalPr { id: p.id.clone(), title: p.title.clone(), branch: p.branch.clone(), base_branch: p.base_branch.clone(), session_id: p.session_id.clone(), state: state.into(), published_number: p.published_number, published_url: p.published_url.clone() }
}

struct Item<'a> {
    repo: &'a str,
    number: Option<u64>,
    folder: Option<&'a ExtFolder>,
    workspace: Option<&'a str>,
    head: Option<&'a str>,
    head_branch: Option<&'a str>,
}

fn state(d: &Data, uses: &[(u64, String)], it: &Item) -> State {
    let named: HashMap<&str, u64> = uses.iter().filter(|(n, _)| it.number.is_none_or(|x| x == *n)).map(|(n, s)| (s.as_str(), *n)).collect();
    let mut agents: Vec<ExtAgent> = d
        .sessions
        .iter()
        .filter(|s| it.workspace.is_some() && d.session_workspace(s) == it.workspace)
        .filter_map(|s| {
            let by_ref = s.launch_spec.github_refs.iter().find(|r| same(&r.repo, it.repo) && it.number.is_none_or(|n| r.number == Some(n))).map(|r| r.number.unwrap_or(0));
            let number = by_ref.or_else(|| named.get(s.id.as_str()).copied());
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
                number: number.filter(|n| *n > 0),
                since: s.status_since,
            })
        })
        .collect();
    agents.sort_by_key(|a| std::cmp::Reverse(a.since));
    agents.truncate(20);
    let suggestions: Vec<ExtSuggestion> = d
        .suggestions
        .iter()
        .filter_map(|s| {
            let SuggestionTarget::GitHub { repo, number } = &s.target else { return None };
            if !same(repo, it.repo) || it.number.is_some_and(|n| n != *number) {
                return None;
            }
            let comments = s.comments.len() as u32;
            let outdated = match (&s.head, it.head) {
                (Some(h), Some(head)) if it.number.is_some() && h != head => comments,
                _ => 0,
            };
            Some(ExtSuggestion { id: s.id.clone(), kind: s.kind.clone(), number: *number, head: s.head.clone(), comments, outdated, status: s.status.clone(), session_id: s.session_id.clone() })
        })
        .collect();
    let local_prs: Vec<ExtLocalPr> = match it.folder {
        Some(f) => d
            .predrafts
            .iter()
            .filter(|p| p.project_id == f.id && !p.archived)
            .filter(|p| match it.number {
                Some(n) => p.published_number == Some(n) || it.head_branch.is_some_and(|b| p.remote_branch.as_deref() == Some(b) || p.branch == b),
                None => p.is_open(),
            })
            .map(local_pr)
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

/// The default bot: the folder's setting, else the last bot used on the repository, else the workspace's Ask bot.
async fn default_bot(app: &App, project: Option<&Project>, repo: Option<&str>, workspace: Option<&str>) -> Option<ExtBot> {
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
        local_prs: vec![],
        branches: vec![],
        today: vec![],
        inbox: None,
        inbox_unread: 0,
        defaults: ExtDefaults { bot: default_bot(app, project, page.repo.as_deref(), ws.as_deref()).await },
        page: page.clone(),
    };
    let Some(repo) = page.repo.as_deref() else { return Ok(out) };
    let uses = launchpad_core::ref_use::sessions(app, repo).await.unwrap_or_default();
    let item = Item { repo, number: page.number, folder: folder.as_ref(), workspace: ws.as_deref(), head: page.head.as_deref(), head_branch: page.head_branch.as_deref() };
    let s = state(&d, &uses, &item);
    (out.agents, out.suggestions, out.local_prs, out.today) = (s.agents, s.suggestions, s.local_prs, s.today);
    out.branches = branches(app, folder.as_ref(), page.number).await;
    let threads = crate::inbox::of(app).threads_of(app, Some(repo), &[]).await.unwrap_or_default();
    out.inbox_unread = threads.iter().filter(|t| t.unread).count() as u32;
    if let Some(n) = page.number {
        out.inbox = threads.iter().find(|t| t.number == Some(n)).map(|t| ExtInbox { thread: t.id.clone(), unread: t.unread, reason: t.reason.clone() });
    }
    Ok(out)
}

/// The row states of a list page: `items` are numbers on a repository's Pull requests or Issues page, thread IDs on
/// Notifications (at most 100).
pub async fn rows(app: &App, url: &str, items: &[String], workspace: Option<&str>) -> Result<ExtRows> {
    let d = Data::load(app).await?;
    let page = parse_page(url);
    let asked = workspace.filter(|w| d.workspaces.iter().any(|x| x.id == *w));
    let items: Vec<&String> = items.iter().take(100).collect();
    // (item, repo, number) for each row.
    let wanted: Vec<(String, String, Option<u64>)> = match (&page.repo, page.kind) {
        (_, PageKind::Notifications) => {
            let ids: Vec<String> = items.iter().map(|s| s.to_string()).collect();
            let threads = crate::inbox::of(app).threads_of(app, None, &ids).await.unwrap_or_default();
            ids.into_iter().filter_map(|id| threads.iter().find(|t| t.id == id).map(|t| (id.clone(), t.repo.clone(), t.number))).collect()
        }
        (Some(repo), _) => items.iter().filter_map(|s| s.parse::<u64>().ok().map(|n| (s.to_string(), repo.clone(), Some(n)))).collect(),
        (None, _) => vec![],
    };
    // Per repository: its folder, the items its sessions named and its Inbox threads.
    type PerRepo = (Option<ExtFolder>, Vec<(u64, String)>, Vec<crate::inbox::Thread>);
    let mut by_repo: HashMap<String, PerRepo> = HashMap::new();
    let mut rows = vec![];
    for (item, repo, number) in wanted {
        let key = repo.to_lowercase();
        if !by_repo.contains_key(&key) {
            let folder = folders(app, &d, &repo, asked).await.into_iter().next();
            let uses = launchpad_core::ref_use::sessions(app, &repo).await.unwrap_or_default();
            let threads = crate::inbox::of(app).threads_of(app, Some(&repo), &[]).await.unwrap_or_default();
            by_repo.insert(key.clone(), (folder, uses, threads));
        }
        let (folder, uses, threads) = &by_repo[&key];
        let ws = folder.as_ref().map(|f| f.workspace_id.as_str()).or(asked);
        // A notification about something else than an issue or pull request (a release, a discussion) has no state.
        let s = match number {
            Some(n) => state(&d, uses, &Item { repo: &repo, number: Some(n), folder: folder.as_ref(), workspace: ws, head: None, head_branch: None }),
            None => State { agents: vec![], suggestions: vec![], local_prs: vec![], today: vec![] },
        };
        let unread = number.and_then(|n| threads.iter().find(|t| t.number == Some(n))).map(|t| t.unread);
        rows.push(ExtRow { item, repo: Some(repo.clone()), number, agents: s.agents, suggestions: s.suggestions, local_prs: s.local_prs, today: s.today, unread });
    }
    let folder = page.repo.as_ref().and_then(|r| by_repo.get(&r.to_lowercase())).and_then(|x| x.0.clone());
    let folder = match (folder, &page.repo) {
        (Some(f), _) => Some(f),
        (None, Some(r)) => folders(app, &d, r, asked).await.into_iter().next(),
        (None, None) => None,
    };
    let ws = folder.as_ref().map(|f| f.workspace_id.clone()).or(asked.map(str::to_string));
    Ok(ExtRows { workspace: ws.as_deref().and_then(|w| d.workspace(w)), folder, page, rows })
}

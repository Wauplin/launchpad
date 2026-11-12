//! Today: the decision list. Home computes, once for every window, phone and count, what waits for the owner in
//! each workspace: questions, finished agents, Local PRs, proposals, drafts to send, GitHub suggestions and review
//! requests, failed routines and outdated runners. The core adds its own records (sessions, Local PRs, runners);
//! the crates above it add theirs through `ext::Extensions::today`. Each item names its main action from the
//! action list (`actions::ACTIONS`). A bus event (`today`/`changed`) tells windows when the list changed.
//!
//! One card per session: the Local PRs of an agent that needs you go inside its card (`Item::grouped`).
//!
//! The same predicates as before decide membership: `Session::needs_you()` for agents, waiting proposals, Outbox
//! drafts and unposted suggestions. Handled items of the last 30 days form the Done list.
use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::App;
use crate::model::{Project, Session, SessionStatus};
use crate::util::now_ms;

/// The filter chips of the Today page, in the order decisions are made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "TodayFilter")]
pub enum Filter {
    /// Open questions of agents.
    Answer,
    /// Finished, failed or interrupted agents not seen yet, and Local PRs to publish.
    Review,
    /// Proposal cards.
    Approve,
    /// Outbox drafts for other people.
    Send,
    /// Suggestions to post and review requests.
    Github,
    /// Failed routines, outdated runners, Home updates and agent CLI updates that did not install.
    Fix,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "TodayKind")]
pub enum Kind {
    Question,
    Finished,
    Failed,
    LocalPr,
    Proposal,
    Outbox,
    Suggestion,
    ReviewRequest,
    Routine,
    Runner,
    /// A newer Launchpad for Home's Space (`launchpad_setup::home_update`).
    HomeUpdate,
    /// An agent CLI update on the runners (`launchpad_setup::cli_update`): a major version to approve, an update
    /// that completed, or one that did not install.
    CliUpdate,
    /// A bot channel or DM where a bot asks the owner (`--ask-owner`) or mentions the owner (`@owner`): one per
    /// channel, until the owner posts there or reads past the newest ask (`launchpad_channels::today`).
    ChannelAsk,
}

/// Where an item belongs, resolved by `compute` into `Item::workspaces`. A window shows only its workspace: an
/// item whose place resolves to no workspace (a repository without a folder, a session outside every workspace)
/// is not listed. Only what Launchpad shares between workspaces shows in every one.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Place {
    /// Every workspace: what is shared (a runner, a tool or preset proposal without a workspace, an Outbox message
    /// without a session).
    #[default]
    Everywhere,
    /// None: not listed.
    Workspace(Option<String>),
    Project(String),
    Session(String),
    /// `owner/repo`: the workspaces of the folders of that repository; none means the item is not listed.
    Repo(String),
}

impl Place {
    /// The place of a launch: its folder's workspace, else the workspace it was launched in.
    pub fn of_spec(spec: &crate::model::LaunchSpec) -> Place {
        use crate::model::Target;
        match &spec.target {
            Target::Project { project_id } | Target::Predraft { project_id } => Place::Project(project_id.clone()),
            Target::SessionFolder { session_id } if spec.workspace_id.is_none() => Place::Session(session_id.clone()),
            _ => Place::Workspace(spec.workspace_id.clone()),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[ts(rename = "TodayItem")]
pub struct Item {
    /// Stable: `<kind>:<record id>`, such as `session:<id>` or `outbox:<id>`.
    pub id: String,
    pub kind: Kind,
    pub filter: Filter,
    pub title: String,
    /// One line under the title.
    pub detail: String,
    /// The workspaces whose Today lists it; empty: every workspace.
    pub workspaces: Vec<String>,
    /// The record it is about: a session, proposal, Outbox message, suggestion, Local PR, Inbox thread, routine or runner ID.
    pub record: String,
    /// The session that asked, finished, wrote or drafted it.
    pub session_id: Option<String>,
    /// The main action of its card, from the action list.
    pub action: Option<String>,
    /// `owner/repo#number`: the GitHub issue or pull request it is about.
    pub github: Option<String>,
    /// Unix milliseconds: since when it waits, or when it was handled.
    #[ts(type = "number")]
    pub at: i64,
    /// Done list only: what happened, such as "Sent to team-chat".
    pub outcome: Option<String>,
    /// Items shown inside this card instead of on their own: the Local PRs of an agent that needs you. One card
    /// per session, so each counts once.
    #[serde(default)]
    pub grouped: Vec<Item>,
    /// A channel ask: the newest unanswered message (what Reply answers) and how many wait.
    #[ts(type = "number | null")]
    #[serde(default)]
    pub message: Option<i64>,
    #[serde(default)]
    pub count: Option<u32>,
    #[serde(skip)]
    #[ts(skip)]
    pub place: Place,
}

impl Item {
    pub fn new(kind: Kind, filter: Filter, record: &str, title: impl Into<String>, place: Place, at: i64) -> Item {
        let prefix = match kind {
            Kind::Question | Kind::Finished | Kind::Failed => "session",
            Kind::LocalPr => "local-pr",
            Kind::Proposal => "proposal",
            Kind::Outbox => "outbox",
            Kind::Suggestion => "suggestion",
            Kind::ReviewRequest => "review-request",
            Kind::Routine => "routine",
            Kind::Runner => "runner",
            Kind::HomeUpdate => "home-update",
            Kind::CliUpdate => "cli-update",
            Kind::ChannelAsk => "channel",
        };
        Item { id: format!("{prefix}:{record}"), kind, filter, title: title.into(), detail: String::new(), workspaces: vec![], record: record.into(), session_id: None, action: None, github: None, at, outcome: None, grouped: vec![], message: None, count: None, place }
    }
    pub fn detail(mut self, detail: impl Into<String>) -> Item {
        self.detail = detail.into();
        self
    }
    pub fn session(mut self, id: Option<String>) -> Item {
        self.session_id = id;
        self
    }
    pub fn github(mut self, repo: &str, number: impl std::fmt::Display) -> Item {
        self.github = Some(format!("{repo}#{number}"));
        self
    }
    pub fn action(mut self, name: &'static str) -> Item {
        debug_assert!(crate::actions::get(name).is_some(), "unknown action {name}");
        self.action = Some(name.into());
        self
    }
    /// A handled item, for the Done list.
    pub fn handled(mut self, outcome: impl Into<String>) -> Item {
        self.outcome = Some(outcome.into());
        self
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, TS)]
pub struct Today {
    /// What waits, in the order to decide it.
    pub items: Vec<Item>,
    /// What was handled in the last 30 days, newest first.
    pub done: Vec<Item>,
}

/// Handled items stay in the Done list this long.
const DONE_DAYS: i64 = 30;
const DONE_MAX: usize = 300;

/// The first line of a text, cut for one card line.
pub fn line(text: &str) -> String {
    let first = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let mut out: String = first.chars().take(160).collect();
    if first.chars().count() > 160 {
        out.push('…');
    }
    out
}

/// Unix milliseconds of an RFC 3339 time (GitHub's `updated_at`), 0 when unreadable.
pub fn parse_time(s: &str) -> i64 {
    time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).map(|t| (t.unix_timestamp_nanos() / 1_000_000) as i64).unwrap_or(0)
}

fn session_item(s: &Session) -> Option<Item> {
    if !s.needs_you() {
        return None;
    }
    let place = Place::Session(s.id.clone());
    let detail = s.last_message.as_deref().map(line).unwrap_or_default();
    let item = if s.open_question {
        Item::new(Kind::Question, Filter::Answer, &s.id, &s.title, place, s.status_since).action("session.answer")
    } else if s.status.is_done() {
        Item::new(Kind::Finished, Filter::Review, &s.id, &s.title, place, s.status_since).action("worktree.merge")
    } else {
        let what = if s.status == SessionStatus::Error { "Failed" } else { "Interrupted" };
        Item::new(Kind::Failed, Filter::Review, &s.id, &s.title, place, s.status_since).action("session.resume").detail(what)
    };
    let item = if item.detail.is_empty() { item.detail(detail) } else { item };
    Some(item.session(Some(s.id.clone())))
}

/// Puts each Local PR whose session is in the list into that session's card: a finished agent's main action
/// becomes reviewing its Local PR. The other Local PRs stay cards of their own.
fn group_local_prs(sessions: &mut [Item], local_prs: Vec<Item>) -> Vec<Item> {
    let mut alone = vec![];
    for pr in local_prs {
        match sessions.iter_mut().find(|s| s.session_id.is_some() && s.session_id == pr.session_id) {
            Some(s) => {
                if s.kind == Kind::Finished {
                    s.action = pr.action.clone();
                }
                s.grouped.push(pr);
            }
            None => alone.push(pr),
        }
    }
    alone
}

/// The core's own items: agents that need you (with their Local PRs), Local PRs to publish, runners to update.
async fn core_items(app: &App) -> Vec<Item> {
    let mut out: Vec<Item> = app.sessions.list().iter().filter_map(session_item).collect();
    // A finished agent whose merge stopped (conflict or failure): its main action is Fix merge.
    if out.iter().any(|i| i.kind == Kind::Finished) {
        let stopped: Vec<_> = crate::worktrees::list(app).await.unwrap_or_default().into_iter()
            .filter(|w| w.primary && w.merge.as_ref().is_some_and(|m| m.phase != crate::worktrees::MergePhase::Waiting))
            .collect();
        for i in out.iter_mut().filter(|i| i.kind == Kind::Finished) {
            if stopped.iter().any(|w| w.sessions.last() == i.session_id.as_ref()) {
                i.action = Some("worktree.fix-merge".into());
            }
        }
    }
    match crate::predrafts::list(app).await {
        Ok(list) => {
            let prs = list.into_iter().filter(|p| !p.archived && p.published_url.is_none() && p.merged.is_none()).map(|p| {
                Item::new(Kind::LocalPr, Filter::Review, &p.id, &p.title, Place::Project(p.project_id.clone()), p.created_at)
                    .detail(format!("Local PR · {} → {}", p.branch, p.base_branch))
                    .session(p.session_id.clone())
                    .action("local-pr.publish")
            });
            let alone = group_local_prs(&mut out, prs.collect());
            out.extend(alone);
        }
        Err(e) => tracing::warn!("Today: Local PRs: {e:#}"),
    }
    if app.paths.role != crate::role::Role::Runner {
        match crate::runners::list(app).await {
            Ok(list) => out.extend(list.into_iter().filter(|r| matches!(r.compat, Some(crate::role::Compat::Outdated | crate::role::Compat::Incompatible))).map(|r| {
                let why = if r.compat == Some(crate::role::Compat::Outdated) { "Runs another build: new sessions wait until you update it" } else { "Another protocol: it cannot connect until you update it" };
                Item::new(Kind::Runner, Filter::Fix, &r.runner.id, format!("Update {}", r.runner.name), Place::Everywhere, r.runner.last_seen_at.unwrap_or(0)).detail(why)
            })),
            Err(e) => tracing::warn!("Today: runners: {e:#}"),
        }
    }
    out
}

/// The workspaces of a session: its folder's, or the one of its launch (bots, temporary folders).
fn session_workspace(app: &App, projects: &[Project], id: &str) -> Option<String> {
    let s = app.sessions.get(id)?;
    let by_id = s.project_id.as_ref().and_then(|p| projects.iter().find(|x| &x.id == p));
    let by_path = || projects.iter().find(|p| s.cwd == p.path || s.cwd.starts_with(&format!("{}/", p.path)));
    by_id.or_else(by_path).and_then(|p| p.workspace_id.clone()).or(s.launch_spec.workspace_id.clone())
}

/// The workspaces an item belongs to (empty: every workspace); None: it is not listed (no workspace).
fn resolve(app: &App, projects: &[Project], place: &Place) -> Option<Vec<String>> {
    let one = |w: Option<String>| w.map(|w| vec![w]);
    match place {
        Place::Everywhere => Some(vec![]),
        Place::Workspace(w) => one(w.clone()),
        Place::Project(id) => one(projects.iter().find(|p| &p.id == id).and_then(|p| p.workspace_id.clone())),
        Place::Session(id) => one(session_workspace(app, projects, id)),
        Place::Repo(repo) => {
            let repo = repo.to_lowercase();
            let mut out: Vec<String> = vec![];
            for p in projects {
                let matches = [&p.gh_repo, &p.gh_upstream].iter().any(|r| r.as_deref().is_some_and(|r| r.to_lowercase() == repo));
                if matches && let Some(w) = &p.workspace_id && !out.contains(w) {
                    out.push(w.clone());
                }
            }
            (!out.is_empty()).then_some(out)
        }
    }
}

/// The whole list, every workspace. Home (or a single daemon) only: runners send `/api/today` to Home.
pub async fn compute(app: &App) -> Result<Today> {
    let projects = crate::projects::list(app).await?;
    let mut all = core_items(app).await;
    all.extend(crate::ext::extensions().today(app).await);
    let since = now_ms() - DONE_DAYS * 86_400_000;
    let (mut items, mut done) = (vec![], vec![]);
    for mut item in all {
        let Some(workspaces) = resolve(app, &projects, &item.place) else { continue };
        for g in &mut item.grouped {
            g.workspaces = workspaces.clone();
        }
        item.workspaces = workspaces;
        if item.outcome.is_some() {
            if item.at >= since {
                done.push(item);
            }
        } else {
            items.push(item);
        }
    }
    items.sort_by(|a, b| a.filter.cmp(&b.filter).then(b.at.cmp(&a.at)));
    done.sort_by_key(|i| std::cmp::Reverse(i.at));
    done.truncate(DONE_MAX);
    Ok(Today { items, done })
}

/// Bus topics whose changes can change the list.
const TOPICS: &[&str] = &["sessions", "predrafts", "runners", "projects", "workspaces", "proposals", "outbox", "suggestions", "inbox", "crons", "home_update", "cli_updates", "channels"];

/// What windows show of a waiting item: when it changes, they refetch.
type ItemKey = (String, Vec<String>, Option<String>, String, String, Vec<String>);

/// What of the sessions can change the list: the ones that need you, with their title, question and status.
fn sessions_key(app: &App) -> Vec<(String, String, bool, i64, Option<String>)> {
    let mut key: Vec<_> = app.sessions.list().into_iter().filter(|s| s.needs_you()).map(|s| (s.id, s.title, s.open_question, s.status_since, s.project_id)).collect();
    key.sort();
    key
}

/// Home: recomputes the list after relevant bus events (at most once per 1.5 s) and publishes `today`/`changed`
/// when the waiting items changed, so every window and phone updates at once. Session updates (frequent while
/// agents work) recompute only when the sessions that need you changed.
pub fn spawn(app: &App) {
    use tokio::sync::broadcast::error::{RecvError, TryRecvError};
    if app.paths.role == crate::role::Role::Runner {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move {
        let mut rx = app.bus.subscribe(None).rx;
        let mut last: Option<Vec<ItemKey>> = None;
        let mut last_sessions = None;
        loop {
            // Wait for a relevant event, then let a burst of them settle.
            let mut full = loop {
                match rx.recv().await {
                    Ok(e) => match topic(&e.text) {
                        Some("sessions") => break false,
                        Some(t) if TOPICS.contains(&t) => break true,
                        _ => {}
                    },
                    Err(RecvError::Lagged(_)) => break true,
                    Err(RecvError::Closed) => return,
                }
            };
            tokio::time::sleep(Duration::from_millis(1500)).await;
            loop {
                match rx.try_recv() {
                    Ok(e) => full |= topic(&e.text).is_some_and(|t| t != "sessions" && TOPICS.contains(&t)),
                    Err(TryRecvError::Lagged(_)) => full = true,
                    Err(_) => break,
                }
            }
            let sessions = sessions_key(&app);
            if !full && last_sessions.as_ref() == Some(&sessions) {
                continue;
            }
            let today = match compute(&app).await {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!("Today: {e:#}");
                    continue;
                }
            };
            last_sessions = Some(sessions);
            let key: Vec<_> = today.items.iter().map(|i| (i.id.clone(), i.workspaces.clone(), i.action.clone(), i.title.clone(), i.detail.clone(), i.grouped.iter().map(|g| g.id.clone()).collect())).collect();
            if last.as_ref() != Some(&key) {
                last = Some(key);
                app.bus.publish("today", "changed", serde_json::json!({ "count": today.items.len() }));
            }
        }
    });
}

/// `{"bus":…,"seq":…,"topic":"<topic>",…}`: reads the topic without parsing the payload.
fn topic(text: &str) -> Option<&str> {
    text.split_once("\"topic\":\"").and_then(|(_, r)| r.split('"').next())
}

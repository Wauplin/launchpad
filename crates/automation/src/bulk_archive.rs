//! Home: Archive automation threads, the owner's bulk archive of a workspace's finished automation threads (the
//! Done backlog that routine runs and bot-to-bot handoffs leave). A preview lists what would go (`preview`); the
//! owner confirms and `apply` archives exactly those that are still eligible, with the same semantics as the
//! thread and session Archive (traces kept, Unarchive brings each one back).
//!
//! Eligible (`origin`, `blocker`, then `task_archive::waits`): sessions Done (read or unread) that automation started,
//! never the owner's chats or launches:
//! - a bot's thread whose key comes from automation (`routine:`, `channel:` for channels and DMs, `agent:`, `hook:`),
//!   or a thread that joined such a thread's conversation (`conversation:`, judged by the conversation's first thread);
//! - an agent session that is not a bot's, started by a routine, a bot, another agent, a hook or a channel without the
//!   owner's click.
//!
//! Never: a question or permission request, sleeping, working, failed, stopped, quota paused, waiting to retry, work
//! not saved on Home, Done less than a minute ago, and anything `task_archive::waits` keeps (the owner took part, a
//! follow-up, a waiting card or report, Suggestion, Outbox message, Local PR or unfinished worktree). Archiving a thread
//! archives its whole conversation, so a conversation goes only when every open thread of it is eligible.
use std::collections::{BTreeMap, HashSet};

use anyhow::{Context, Result, bail};
use launchpad_core::App;
use launchpad_core::bot_threads::{self, BotThread};
use launchpad_core::model::{SenderKind, Session, SessionStatus};
use launchpad_core::util::now_ms;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// What the owner narrows the list to.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Filter {
    /// A bot of the workspace (ID, slug or name): conversations one of its threads is part of.
    pub bot: Option<String>,
    /// Done for at least this long (45m, 2h, 3d).
    pub older_than: Option<String>,
}

/// One thread or session that would be (or was) archived.
#[derive(Clone, Debug, Serialize, TS, PartialEq)]
pub struct ArchiveItem {
    pub session_id: String,
    pub bot_id: Option<String>,
    pub bot_name: Option<String>,
    pub thread_id: Option<String>,
    /// The thread's title, else the session's.
    pub title: String,
    /// What started it: routine, channel (channels and DMs), agent, hook or bot.
    pub origin: String,
    /// `unread` (Done, not seen yet) or `done`.
    pub state: String,
    /// When it became Done (Unix ms).
    #[ts(type = "number")]
    pub done_at: i64,
}

/// An automation thread that stays, and why.
#[derive(Clone, Debug, Serialize, TS, PartialEq)]
pub struct Kept {
    pub session_id: String,
    pub bot_name: Option<String>,
    pub title: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, TS)]
pub struct ArchivePreview {
    pub workspace_id: String,
    pub count: usize,
    /// Newest Done first; the threads of one conversation together.
    pub items: Vec<ArchiveItem>,
    /// Automation threads of the workspace that stay (owner threads are not listed).
    pub kept: Vec<Kept>,
}

#[derive(Clone, Debug, Serialize, TS)]
pub struct ArchiveOutcome {
    /// "Archived N automation threads" (the activity list's name for it).
    pub title: String,
    pub archived: Vec<ArchiveItem>,
    /// Asked for but no longer eligible, or changed since the checks: nothing happened to them.
    pub kept: Vec<Kept>,
    /// The archive was tried and its outcome is not known: never retried, check them by hand.
    pub failed: Vec<Kept>,
}

/// Pure: what started a session, None for the owner's (their chats, Telegram, GitHub item and folio threads, their
/// launches and accepted cards). `thread` is a bot session's thread row, `main` the first thread of its conversation.
pub fn origin(s: &Session, thread: Option<&BotThread>, main: Option<&BotThread>) -> Option<&'static str> {
    fn of_key(key: Option<&str>) -> Option<&'static str> {
        let prefix = key?.split_once(':')?.0;
        match prefix {
            "routine" => Some("routine"),
            "channel" => Some("channel"),
            "agent" => Some("agent"),
            "hook" => Some("hook"),
            _ => None,
        }
    }
    if s.launch_spec.bot.is_some() {
        let key = thread?.key.as_deref();
        if key.is_some_and(|k| k.starts_with("conversation:")) {
            // Joined another bot's conversation: the conversation's first thread decides.
            return main.filter(|m| !m.key.as_deref().is_some_and(|k| k.starts_with("conversation:"))).and_then(|m| of_key(m.key.as_deref()));
        }
        return of_key(key);
    }
    if s.launch_spec.routine.is_some() {
        return Some("routine");
    }
    let from = s.launch_spec.from.as_ref().filter(|f| !f.accepted)?;
    match from.kind {
        SenderKind::Routine => Some("routine"),
        SenderKind::Bot => Some("bot"),
        SenderKind::Session => Some("agent"),
        SenderKind::Hook => Some("hook"),
        SenderKind::Channel => Some("channel"),
        SenderKind::Wake => None,
    }
}

/// Pure: why the session's state keeps it, None when it is a clean Done for at least `min_age` ms (and at least
/// `task_archive::SETTLE_MS`, so the records of its last turn reached Home).
pub fn blocker(s: &Session, now: i64, min_age: i64) -> Option<String> {
    use SessionStatus::*;
    let why = match s.status {
        Starting | Working | Background => "working",
        QuotaPaused => "quota paused",
        Error | Interrupted => "failed or interrupted",
        Stopped => "stopped",
        InTerminal => "open in a terminal",
        Closed => "already archived",
        Waiting if s.open_question => "asks you a question",
        Waiting if s.sleep.is_some() => "sleeping",
        Waiting if s.quota_pause.is_some() => "quota paused",
        Waiting if s.capacity_retry_at.is_some() => "waits to retry",
        Waiting if s.unsaved.is_some() => "its work is not saved on Home yet",
        Waiting => {
            let age = now - s.status_since;
            if age < crate::task_archive::SETTLE_MS {
                "Done less than a minute ago"
            } else if age < min_age {
                "Done more recently than the age filter"
            } else {
                return None;
            }
        }
    };
    Some(why.into())
}

struct Member {
    session: Session,
    bot: Option<(String, String)>,
    thread: Option<BotThread>,
    origin: &'static str,
}

impl Member {
    fn item(&self) -> ArchiveItem {
        ArchiveItem {
            session_id: self.session.id.clone(),
            bot_id: self.bot.as_ref().map(|b| b.0.clone()),
            bot_name: self.bot.as_ref().map(|b| b.1.clone()),
            thread_id: self.thread.as_ref().map(|t| t.id.clone()),
            title: self.thread.as_ref().map_or_else(|| self.session.title.clone(), |t| t.title.clone()),
            origin: self.origin.into(),
            state: if self.session.needs_you() { "unread" } else { "done" }.into(),
            done_at: self.session.status_since,
        }
    }
    fn kept(&self, reason: String) -> Kept {
        Kept { session_id: self.session.id.clone(), bot_name: self.bot.as_ref().map(|b| b.1.clone()), title: self.item().title, reason }
    }
}

/// A conversation (bot threads) or one session, with why it stays (None: eligible).
struct Group {
    members: Vec<Member>,
    reasons: Vec<Option<String>>,
}

impl Group {
    fn eligible(&self) -> bool {
        self.reasons.iter().all(Option::is_none)
    }
    fn kept(&self) -> Vec<Kept> {
        let first = self.reasons.iter().flatten().next().cloned().unwrap_or_default();
        self.members.iter().zip(&self.reasons).map(|(m, r)| m.kept(r.clone().unwrap_or_else(|| format!("another thread of its conversation: {first}")))).collect()
    }
}

/// The workspace's ID from its ID or name.
pub async fn workspace_id(app: &App, workspace: &str) -> Result<String> {
    let w = workspace.trim();
    if w.is_empty() || w == "all" {
        bail!("name a workspace: the archive covers one workspace");
    }
    launchpad_core::workspace::list(app).await?.into_iter().find(|x| x.id == w || x.name.eq_ignore_ascii_case(w)).map(|x| x.id).with_context(|| format!("no workspace {w}"))
}

async fn groups(app: &App, ws: &str, filter: &Filter) -> Result<Vec<Group>> {
    let min_age = match filter.older_than.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(t) => i64::from(crate::schedules::parse_minutes(t).map_err(|_| anyhow::anyhow!("older_than takes minutes, hours or days: 45m, 2h, 3d"))?) * 60_000,
        None => 0,
    };
    let bots: Vec<launchpad_core::bots::Bot> = launchpad_core::bots::list(app).await?.into_iter().filter(|b| b.workspace_id == ws).collect();
    let only_bot = match filter.bot.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(b) => Some(bots.iter().find(|x| x.id == b || x.slug.eq_ignore_ascii_case(b) || x.settings.name.eq_ignore_ascii_case(b)).map(|x| x.id.clone()).with_context(|| format!("no bot {b} in this workspace"))?),
        None => None,
    };
    let threads = bot_threads::all(app).await?;
    let now = now_ms();
    let mut by_group: BTreeMap<String, Vec<Member>> = BTreeMap::new();
    for s in app.sessions.list() {
        if s.status == SessionStatus::Closed || launchpad_core::scope::of_session(app, &s.id).await.as_deref() != Some(ws) {
            continue;
        }
        let (bot, thread, main) = match &s.launch_spec.bot {
            Some(b) => {
                let Some(t) = threads.get(&(b.id.clone(), b.thread_id().to_string())) else { continue };
                // An older session of a thread (threads from before a thread was one session) or an archived row.
                if t.archived_at.is_some() || bot_threads::session(app, &b.id, &t.id).is_some_and(|newest| newest.id != s.id) {
                    continue;
                }
                let main = t.conversation.split_once(':').and_then(|(mb, mt)| threads.get(&(mb.to_string(), mt.to_string())));
                let name = bots.iter().find(|x| x.id == b.id).map_or_else(|| b.name.clone(), |x| x.settings.name.clone());
                (Some((b.id.clone(), name)), Some(t.clone()), main)
            }
            None => (None, None, None),
        };
        let Some(origin) = origin(&s, thread.as_ref(), main) else { continue };
        let key = thread.as_ref().map_or_else(|| format!("session:{}", s.id), |t| format!("conversation:{}", t.conversation));
        by_group.entry(key).or_default().push(Member { session: s, bot, thread, origin });
    }
    let mut out = vec![];
    for (key, members) in by_group {
        if let Some(b) = &only_bot
            && !members.iter().any(|m| m.bot.as_ref().is_some_and(|x| &x.0 == b))
        {
            continue;
        }
        let mut reasons = vec![];
        for m in &members {
            let why = match blocker(&m.session, now, min_age) {
                Some(why) => Some(why),
                None => match crate::task_archive::waits(app, &m.session).await {
                    Ok(why) => why,
                    Err(e) => Some(format!("a check could not be read: {e:#}")),
                },
            };
            reasons.push(why);
        }
        // Archiving one thread archives its conversation: every open thread of it must be in this group.
        if let Some(conversation) = key.strip_prefix("conversation:") {
            let ids: HashSet<&str> = members.iter().map(|m| m.session.id.as_str()).collect();
            for t in bot_threads::conversation_threads(app, conversation).await? {
                if t.archived_at.is_none()
                    && let Some(open) = bot_threads::open_session(app, &t.bot_id, &t.id)
                    && !ids.contains(open.id.as_str())
                {
                    reasons[0].get_or_insert_with(|| format!("its conversation also holds “{}”, which is not an automation thread of this workspace", t.title));
                }
            }
        }
        out.push(Group { members, reasons });
    }
    // Newest Done first.
    out.sort_by_key(|g| std::cmp::Reverse(g.members.iter().map(|m| m.session.status_since).max().unwrap_or(0)));
    Ok(out)
}

/// What Archive automation threads would archive in workspace `ws` now, and the automation threads that stay.
pub async fn preview(app: &App, ws: &str, filter: &Filter) -> Result<ArchivePreview> {
    let groups = groups(app, ws, filter).await?;
    let mut items = vec![];
    let mut kept = vec![];
    for g in &groups {
        if g.eligible() {
            items.extend(g.members.iter().map(Member::item));
        } else {
            kept.extend(g.kept());
        }
    }
    Ok(ArchivePreview { workspace_id: ws.to_string(), count: items.len(), items, kept })
}

/// One pass at a time.
static APPLY: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Archives the sessions of `ids` (the preview the owner confirmed) that are still eligible, each conversation
/// whole: every session is archived only while still Done since the checks (`runners::archive_if_done`), then its
/// thread rows. An uncertain archive is reported, never retried.
pub async fn apply(app: &App, ws: &str, filter: &Filter, ids: &[String]) -> Result<ArchiveOutcome> {
    let _one = APPLY.lock().await;
    let wanted: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let groups = groups(app, ws, filter).await?;
    let (mut archived, mut kept, mut failed) = (vec![], vec![], vec![]);
    let mut seen: HashSet<String> = HashSet::new();
    for g in groups {
        let asked = g.members.iter().filter(|m| wanted.contains(m.session.id.as_str())).count();
        seen.extend(g.members.iter().map(|m| m.session.id.clone()));
        if asked == 0 {
            continue;
        }
        if !g.eligible() {
            kept.extend(g.kept().into_iter().filter(|k| wanted.contains(k.session_id.as_str())));
            continue;
        }
        if asked < g.members.len() {
            kept.extend(g.members.iter().map(|m| m.kept("its conversation has threads that were not in the preview: preview again".into())));
            continue;
        }
        let mut done = vec![];
        let mut stopped = false;
        for m in &g.members {
            match launchpad_core::runners::archive_if_done(app, &m.session.id, m.session.status_since).await {
                Ok(true) => done.push(m.item()),
                Ok(false) => {
                    kept.push(m.kept("it changed since the checks".into()));
                    stopped = true;
                }
                Err(e) => {
                    failed.push(m.kept(format!("{e:#}")));
                    stopped = true;
                }
            }
            if stopped {
                break;
            }
        }
        // The thread rows follow once every session of the conversation is archived (a thread whose session is
        // archived already counts as archived).
        if !stopped
            && let Some((bot, t)) = g.members.iter().find_map(|m| Some((m.bot.as_ref()?.0.clone(), m.thread.as_ref()?.id.clone())))
            && let Err(e) = bot_threads::archive(app, &bot, &t, true).await
        {
            tracing::warn!("archive automation threads: thread {t} of bot {bot}: {e:#}");
        }
        archived.extend(done);
    }
    for id in ids.iter().filter(|id| !seen.contains(id.as_str())) {
        let title = app.sessions.get(id).map(|s| s.title).unwrap_or_default();
        kept.push(Kept { session_id: id.clone(), bot_name: None, title, reason: "not an open automation thread of this workspace".into() });
    }
    let n = archived.len();
    tracing::info!("archive automation threads in workspace {ws}: {n} archived, {} kept, {} uncertain", kept.len(), failed.len());
    Ok(ArchiveOutcome { title: format!("Archived {n} automation thread{}", if n == 1 { "" } else { "s" }), archived, kept, failed })
}

#[cfg(test)]
mod tests {
    use super::*;
    use launchpad_core::model::{BotLaunch, MessageFrom, RoutineLaunch};

    fn session(status: SessionStatus) -> Session {
        launchpad_harness::install();
        serde_json::from_value(serde_json::json!({
            "id": "s1", "harness": "codex", "cwd": "/tmp/x", "title": "Run", "status": status, "status_since": 1_000,
            "live": false, "open_question": false, "prompt": "p", "usage": {"input_tokens": 0, "output_tokens": 0, "cached_tokens": 0},
            "started_at": 0, "last_seq": 1, "launch_spec": {}, "seen_at": 0
        }))
        .unwrap()
    }

    fn bot_session() -> Session {
        let mut s = session(SessionStatus::Waiting);
        s.launch_spec.bot = Some(BotLaunch { id: "b".into(), thread: "t".into(), ..Default::default() });
        s
    }

    fn thread(id: &str, key: Option<&str>, conversation: &str) -> BotThread {
        BotThread { id: id.into(), bot_id: "b".into(), key: key.map(str::to_string), title: "T".into(), created_at: 0, archived_at: None, conversation: conversation.into(), renamed: false, run: None }
    }

    fn from(kind: SenderKind, accepted: bool) -> Option<MessageFrom> {
        Some(MessageFrom { kind, id: "x".into(), name: "x".into(), hops: 1, accepted, ..Default::default() })
    }

    #[test]
    fn bot_threads_count_by_their_key() {
        let s = bot_session();
        for (key, want) in [("routine:nightly.toml", "routine"), ("channel:c1", "channel"), ("agent:s9", "agent"), ("hook:h1", "hook")] {
            assert_eq!(origin(&s, Some(&thread("t", Some(key), "b:t")), None), Some(want), "{key}");
        }
        // The owner's chat, Telegram, a GitHub item's or a folio's thread: never.
        for key in [None, Some("telegram"), Some("github:o/r#1"), Some("folio:f1")] {
            assert_eq!(origin(&s, Some(&thread("t", key, "b:t")), None), None, "{key:?}");
        }
        // No thread row: kept.
        assert_eq!(origin(&s, None, None), None);
    }

    #[test]
    fn a_joined_conversation_counts_by_its_first_thread() {
        let s = bot_session();
        let joined = thread("t", Some("conversation:a:m"), "a:m");
        // A routine run's handoff to another bot.
        assert_eq!(origin(&s, Some(&joined), Some(&thread("m", Some("routine:daily.toml"), "a:m"))), Some("routine"));
        // A bot the owner's chat brought in: the owner's.
        assert_eq!(origin(&s, Some(&joined), Some(&thread("m", None, "a:m"))), None);
        // Unknown first thread: kept.
        assert_eq!(origin(&s, Some(&joined), None), None);
    }

    #[test]
    fn task_sessions_count_by_their_sender() {
        let mut run = session(SessionStatus::Waiting);
        run.launch_spec.routine = Some(RoutineLaunch { id: "r1".into(), ..Default::default() });
        assert_eq!(origin(&run, None, None), Some("routine"));
        let mut s = session(SessionStatus::Waiting);
        assert_eq!(origin(&s, None, None), None, "the owner's launch");
        for (kind, want) in [(SenderKind::Bot, Some("bot")), (SenderKind::Session, Some("agent")), (SenderKind::Hook, Some("hook")), (SenderKind::Channel, Some("channel")), (SenderKind::Wake, None)] {
            s.launch_spec.from = from(kind, false);
            assert_eq!(origin(&s, None, None), want, "{kind:?}");
        }
        // A card the owner accepted is the owner's.
        s.launch_spec.from = from(SenderKind::Bot, true);
        assert_eq!(origin(&s, None, None), None);
    }

    #[test]
    fn only_a_clean_settled_done_goes() {
        let s = session(SessionStatus::Waiting);
        let now = s.status_since + crate::task_archive::SETTLE_MS;
        assert_eq!(blocker(&s, now, 0), None);
        // Unread is still Done: it goes.
        assert!(s.needs_you() && blocker(&s, now, 0).is_none());
        assert!(blocker(&s, s.status_since + 1, 0).is_some(), "too fresh");
        assert!(blocker(&s, now, crate::task_archive::SETTLE_MS + 1).is_some(), "age filter");
        for status in [SessionStatus::Starting, SessionStatus::Working, SessionStatus::Background, SessionStatus::QuotaPaused, SessionStatus::Error, SessionStatus::Interrupted, SessionStatus::Stopped, SessionStatus::InTerminal, SessionStatus::Closed] {
            assert!(blocker(&session(status), now, 0).is_some(), "{status:?}");
        }
        let mut asking = s.clone();
        asking.open_question = true;
        assert_eq!(blocker(&asking, now, 0).as_deref(), Some("asks you a question"));
        let mut sleeping: Session = serde_json::from_value({
            let mut v = serde_json::to_value(&s).unwrap();
            v["sleep"] = serde_json::json!({"until": 5, "waits": "", "note": "", "schedule_id": "x"});
            v
        })
        .unwrap();
        assert_eq!(blocker(&sleeping, now, 0).as_deref(), Some("sleeping"));
        sleeping.sleep = None;
        sleeping.capacity_retry_at = Some(now + 1);
        assert!(blocker(&sleeping, now, 0).is_some());
    }
}

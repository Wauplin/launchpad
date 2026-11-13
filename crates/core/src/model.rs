//! Types shared by the session manager, the harness adapters, the API and the UI (exported with ts-rs).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

/// A harness-owned background command. IDs are native, never arbitrary OS PIDs.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct SessionProcess {
    pub thread_id: String,
    pub process_id: String,
    pub command: String,
    pub cwd: String,
    pub output: String,
}

/// A registered harness ID. Adding an adapter does not change this type or its wire format.
#[derive(Clone, Copy, Debug, Serialize, TS, PartialEq, Eq, Hash)]
#[ts(type = "string")]
pub struct HarnessKind(&'static str);

impl HarnessKind {
    pub const fn new(id: &'static str) -> Self {
        Self(id)
    }
    pub fn as_str(self) -> &'static str {
        self.0
    }
    pub fn display(self) -> &'static str {
        crate::harness::registration(self).descriptor.name
    }
    pub fn parse(id: &str) -> Option<Self> {
        crate::harness::registrations()
            .iter()
            .find(|r| r.descriptor.id.as_str() == id)
            .map(|r| r.descriptor.id)
    }
}

impl<'de> Deserialize<'de> for HarnessKind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let id = String::deserialize(deserializer)?;
        Self::parse(&id)
            .ok_or_else(|| serde::de::Error::custom(format!("unregistered harness: {id}")))
    }
}

/// Session states, the same for every harness.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Starting,
    Working,
    /// Done: the turn finished and the agent waits for input.
    Waiting,
    /// Running: the agent only waits for a background command.
    Background,
    Closed,
    InTerminal,
    /// The owner stopped an unfinished turn. Resumable, but not Done.
    Stopped,
    /// Included subscription quota exhausted; continuation is held locally.
    QuotaPaused,
    /// The daemon stopped during a turn. Resumable.
    Interrupted,
    Error,
}

impl SessionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionStatus::Starting => "starting",
            SessionStatus::Working => "working",
            SessionStatus::Waiting => "waiting",
            SessionStatus::Background => "background",
            SessionStatus::Closed => "closed",
            SessionStatus::InTerminal => "in_terminal",
            SessionStatus::Stopped => "stopped",
            SessionStatus::QuotaPaused => "quota_paused",
            SessionStatus::Interrupted => "interrupted",
            SessionStatus::Error => "error",
        }
    }
    pub fn parse(s: &str) -> SessionStatus {
        match s {
            "starting" => SessionStatus::Starting,
            "working" => SessionStatus::Working,
            "waiting" => SessionStatus::Waiting,
            "background" => SessionStatus::Background,
            "closed" => SessionStatus::Closed,
            "in_terminal" => SessionStatus::InTerminal,
            "stopped" => SessionStatus::Stopped,
            "quota_paused" => SessionStatus::QuotaPaused,
            "interrupted" => SessionStatus::Interrupted,
            _ => SessionStatus::Error,
        }
    }
    pub fn is_running(self) -> bool {
        matches!(self, SessionStatus::Starting | SessionStatus::Working | SessionStatus::Background)
    }
    pub fn is_done(self) -> bool {
        matches!(self, SessionStatus::Waiting)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, TS, PartialEq)]
pub struct Usage {
    /// Tokens in the context of the last model call.
    pub context_tokens: Option<u64>,
    pub context_window: Option<u64>,
    /// Totals for the session.
    pub input_tokens: u64,
    pub cached_tokens: u64,
    pub output_tokens: u64,
    /// Reported cumulative USD cost when the harness exposes it; not necessarily a billed charge.
    pub cost_usd: Option<f64>,
    /// Native pricing is unknown; no cost or cost protection is available.
    #[serde(default)]
    pub cost_unavailable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Session {
    pub id: String,
    pub harness: HarnessKind,
    /// On Home: the runner that runs this session. None: this daemon runs it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub runner_id: Option<String>,
    /// Increases each time the session moves to another runner. Home refuses changes from an older generation.
    #[serde(default)]
    pub generation: u32,
    /// Opaque native conversation ID, interpreted only by its adapter.
    pub harness_session_id: Option<String>,
    pub project_id: Option<String>,
    pub cwd: String,
    pub title: String,
    /// Internal durable naming state; owner edits cancel pending generation.
    #[serde(skip)]
    #[ts(skip)]
    pub title_pending: bool,
    /// The title was already renamed from the conversation at a turn end. That happens once, at the first
    /// turn end: follow-up turns keep the title.
    #[serde(default)]
    #[ts(skip)]
    pub title_refreshed: bool,
    pub status: SessionStatus,
    /// The session was Done when archived; retained only while Closed.
    #[serde(default)]
    pub archived_done: bool,
    /// When the status last changed (Unix ms).
    #[ts(type = "number")]
    pub status_since: i64,
    /// A harness process is attached. A done session without a process resumes when you send a message.
    pub live: bool,
    /// The agent asked a question that is not answered yet.
    pub open_question: bool,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub prompt: String,
    pub usage: Usage,
    #[ts(type = "number")]
    pub started_at: i64,
    #[ts(type = "number | null")]
    pub ended_at: Option<i64>,
    #[ts(type = "number")]
    pub last_seq: i64,
    pub launch_spec: LaunchSpec,
    #[serde(default)]
    pub quota_pause: Option<QuotaPause>,
    /// Lifetime automatic capacity retries claimed for this session (never reset by manual input).
    #[serde(default)]
    pub capacity_retry_count: u8,
    /// Pending automatic continuation deadline, Unix milliseconds.
    #[serde(default)]
    #[ts(type = "number | null")]
    pub capacity_retry_at: Option<i64>,
    /// When a focused window last showed the session (Unix ms, 0: never). Unseen while older than `status_since`.
    #[ts(type = "number")]
    pub seen_at: i64,
    /// Start of the last assistant message, as one line of plain text.
    pub last_message: Option<String>,
    /// The work of the last turn (branches, plain folders, conversation) is not all on Home yet: shown as
    /// saving, retried by its runner until it is (`turn_sync`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub unsaved: Option<Unsaved>,
}

/// A turn whose work has not reached Home yet.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct Unsaved {
    /// When the turn ended (Unix ms).
    #[ts(type = "number")]
    pub since: i64,
    /// Failed attempts so far; 0 while the first one runs.
    pub attempts: u32,
    /// Why the last attempt failed.
    pub note: Option<String>,
}

impl Session {
    /// Archiving a completed turn must not block delayed launches waiting for it.
    /// An open question is not done: the agent waits for the owner.
    pub fn dependency_done(&self) -> bool {
        (self.status.is_done() && !self.open_question) || (self.status == SessionStatus::Closed && self.archived_done)
    }

    pub fn change_status(&mut self, status: SessionStatus) {
        self.archived_done = status == SessionStatus::Closed && self.dependency_done();
        self.status = status;
    }

    /// The session without bulky launch data, for lists and live updates: attachment bytes are dropped
    /// (names stay) and the rendered prompt is cut. `GET /api/sessions/{id}` returns the full record.
    pub fn light(&self) -> Session {
        const PROMPT: usize = 4000;
        let mut s = self.clone();
        if s.prompt.len() > PROMPT {
            let mut end = PROMPT;
            while !s.prompt.is_char_boundary(end) { end -= 1; }
            s.prompt.truncate(end);
            s.prompt.push('…');
        }
        if let Some(bot) = &mut s.launch_spec.bot { bot.instructions.clear(); }
        for a in &mut s.launch_spec.attachments {
            if !a.data_url.starts_with("upload:") { a.data_url.clear(); }
        }
        if let Some(pause) = &mut s.quota_pause {
            for image in &mut pause.images { image.data_url.clear(); }
            for message in &mut pause.messages {
                for image in &mut message.images { image.data_url.clear(); }
            }
        }
        s
    }

    /// The session without inline attachment bytes (data URLs; `upload:` references and names stay): what a runner
    /// replicates to Home. Launches and messages carry references (`attachments::to_refs`), so only older records lose anything.
    pub fn without_bytes(&self) -> Session {
        let mut s = self.clone();
        let inline = |url: &mut String| if url.starts_with("data:") { url.clear() };
        s.launch_spec.attachments.iter_mut().for_each(|a| inline(&mut a.data_url));
        if let Some(pause) = &mut s.quota_pause {
            pause.images.iter_mut().for_each(|i| inline(&mut i.data_url));
            pause.messages.iter_mut().flat_map(|m| m.images.iter_mut()).for_each(|i| inline(&mut i.data_url));
        }
        s
    }

    /// Shared "Needs you" predicate: an open question, or Done, Failed or Interrupted not seen since the status changed.
    pub fn needs_you(&self) -> bool {
        let unseen = self.seen_at < self.status_since;
        (self.status.is_done() && (self.open_question || unseen)) || (matches!(self.status, SessionStatus::Error | SessionStatus::Interrupted) && unseen)
    }

    /// The reported cost reached the session cost limit: no new turn may start.
    pub fn budget_reached(&self) -> bool {
        !self.usage.cost_unavailable && self.launch_spec.budget_usd.is_some_and(|b| self.usage.cost_usd.unwrap_or(0.0) >= b)
    }
}

/// A file selected or pasted into the launch composer.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct FileAttachment {
    pub name: String,
    pub data_url: String,
}

/// An image attached to a user message, kept locally in the event log.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ImageAttachment {
    pub name: String,
    pub data_url: String,
}

/// One normalized event in a session (`session_event` table, streamed over the WebSocket).
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct SessionEvent {
    pub session_id: String,
    /// 0 for live-only events (text deltas, command output), which are not stored.
    #[ts(type = "number")]
    pub seq: i64,
    #[ts(type = "number")]
    pub ts: i64,
    pub body: EventBody,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventBody {
    /// A native child agent. Optional fields update only the metadata supplied by the harness.
    Subagent { agent_id: String, parent_id: Option<String>, name: Option<String>, status: Option<String>, model: Option<String> },
    /// Child traces share the root session's durable sequence and transport, but stay separate in the UI.
    SubagentEvent { agent_id: String, body: Box<EventBody> },
    /// Sent by you, or by `from` (another bot or agent, a routine). `queued` while the agent has not read it yet.
    UserMessage {
        text: String,
        client_id: String,
        queued: bool,
        #[serde(default)]
        images: Vec<ImageAttachment>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        from: Option<MessageFrom>,
    },
    /// The agent read a queued message.
    MessageDelivered { client_id: String },
    /// Live text of an assistant message (not stored; the final text comes as `assistant_message`).
    AssistantDelta { item_id: String, text: String },
    AssistantMessage { item_id: String, text: String },
    Reasoning { item_id: String, text: String },
    /// Upserts tool metadata by item ID; some protocols fill input/title in after the pending event.
    ToolCall {
        item_id: String,
        name: String,
        /// One line, for the collapsed view: the command, the file path, the query.
        summary: String,
        #[ts(type = "unknown")]
        input: Value,
    },
    /// Live output of a running command (not stored).
    ToolOutput { item_id: String, text: String },
    ToolResult {
        item_id: String,
        output: String,
        is_error: bool,
        exit_code: Option<i32>,
        #[ts(type = "number | null")]
        duration_ms: Option<i64>,
    },
    FileChange {
        item_id: String,
        path: String,
        /// `add`, `update` or `delete`.
        change: String,
        /// Unified diff lines (`+`, `-`, ` `), or the content of a new file.
        diff: String,
    },
    Plan { items: Vec<PlanItem> },
    AskUser { request_id: String, questions: Vec<Question> },
    AskAnswered { request_id: String, answers: Vec<String> },
    BackgroundTask {
        task_id: String,
        description: String,
        /// `running`, `completed`, `failed` or `stopped`.
        status: String,
        output_file: Option<String>,
    },
    TurnEnd {
        #[ts(type = "number | null")]
        duration_ms: Option<i64>,
        interrupted: bool,
        error: Option<String>,
    },
    Status { status: SessionStatus },
    Error { message: String },
    /// A line from Launchpad itself ("Resumed", "Opened in terminal").
    Notice { text: String },
}

impl EventBody {
    /// Live-only events are streamed but not stored.
    pub fn is_ephemeral(&self) -> bool {
        if let Self::SubagentEvent { body, .. } = self { return body.is_ephemeral(); }
        matches!(self, EventBody::AssistantDelta { .. } | EventBody::ToolOutput { .. })
    }
    pub fn kind(&self) -> &'static str {
        match self {
            EventBody::Subagent { .. } => "subagent",
            EventBody::SubagentEvent { .. } => "subagent_event",
            EventBody::UserMessage { .. } => "user_message",
            EventBody::MessageDelivered { .. } => "message_delivered",
            EventBody::AssistantDelta { .. } => "assistant_delta",
            EventBody::AssistantMessage { .. } => "assistant_message",
            EventBody::Reasoning { .. } => "reasoning",
            EventBody::ToolCall { .. } => "tool_call",
            EventBody::ToolOutput { .. } => "tool_output",
            EventBody::ToolResult { .. } => "tool_result",
            EventBody::FileChange { .. } => "file_change",
            EventBody::Plan { .. } => "plan",
            EventBody::AskUser { .. } => "ask_user",
            EventBody::AskAnswered { .. } => "ask_answered",
            EventBody::BackgroundTask { .. } => "background_task",
            EventBody::TurnEnd { .. } => "turn_end",
            EventBody::Status { .. } => "status",
            EventBody::Error { .. } => "error",
            EventBody::Notice { .. } => "notice",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct PlanItem {
    pub text: String,
    /// `pending`, `in_progress` or `completed`.
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Question {
    /// Permission choices must be selected explicitly; arbitrary text is not an approval.
    #[serde(default)]
    pub permission: bool,
    pub id: String,
    pub header: String,
    pub question: String,
    pub options: Vec<QuestionOption>,
    pub multi_select: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct QuestionOption {
    pub label: String,
    pub description: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    #[default]
    Yolo,
    Approvals,
    /// The harness routes permission decisions to its native AI reviewer.
    Auto,
}

impl PermissionMode {
    /// How strict the mode is: YOLO, then Auto, then Ask for approval. Only the owner loosens a mode; an agent may
    /// only make one stricter (`agent_auth::permission_guard`, `bots::agent_refusal`, scheduled launches).
    pub fn strictness(self) -> u8 {
        match self {
            PermissionMode::Yolo => 0,
            PermissionMode::Auto => 1,
            PermissionMode::Approvals => 2,
        }
    }
}

/// Context attached to one follow-up message. Folders are always read-only references.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(default)]
pub struct MessageContext {
    pub mentions: Vec<PromptMention>,
    pub skills: Vec<SkillRef>,
    pub context_projects: Vec<ContextProject>,
    pub github_refs: Vec<GitHubRef>,
    pub agent_traces: Vec<AgentTraceRef>,
    /// Who sends it when it is not the owner: Home stamps it from its own records (`senders`), never from a request.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub from: Option<MessageFrom>,
}

/// Who sent a message that is not the owner's, stamped by Home from its own record of the sender (`senders`): another
/// bot or agent (its text arrives under a header naming it, fenced), a routine run or an incoming hook (their own first
/// line says so).
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq, Eq, Default)]
#[serde(default)]
pub struct MessageFrom {
    pub kind: SenderKind,
    /// The bot, session, routine, hook or bot channel ID.
    pub id: String,
    /// The bot's name, the session's title, the routine's title, the hook's name or the channel's ("#release-train",
    /// "Maintainer ↔ Release").
    pub name: String,
    /// A channel message: who wrote it (a bot's name, or "you" for the owner).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub by: Option<String>,
    /// Delivered by this message card (proposal ID).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub card: Option<String>,
    /// The owner clicked the card that delivered it ("from Scout, accepted by you").
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[ts(optional, as = "Option<bool>")]
    pub accepted: bool,
    /// Automatic messages in the chain that caused it (routine runs, agents' messages, cards accepted automatically),
    /// itself included; 0 after an owner message or click (`senders::MAX_HOPS`).
    #[ts(type = "number")]
    pub hops: u32,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SenderKind {
    /// A session of a bot's thread.
    #[default]
    Bot,
    /// An agent session that is not a bot's.
    Session,
    /// A run of a routine.
    Routine,
    /// An incoming hook (`POST /hooks/<id>`); its hops come from `X-Launchpad-Hops`.
    Hook,
    /// A message in a bot channel or DM (`launchpad_channels::channel`), delivered under the channel's own frame; the
    /// channel's loop guard decided its delivery.
    Channel,
}

impl MessageContext {
    /// Nothing to add to the message (mentions only mark text that is already there).
    pub fn is_empty(&self) -> bool {
        self.context_projects.is_empty() && self.github_refs.is_empty() && self.agent_traces.is_empty() && self.skills.is_empty()
    }
}

/// A selected prompt occurrence. Offsets are UTF-16 units, matching textarea selection APIs.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct PromptMention {
    pub id: String,
    pub start: u32,
    pub end: u32,
    pub text: String,
    pub key: String,
    /// True when the attachment exists only because of mentions, not a manual picker selection.
    pub owned: bool,
}

/// A skill explicitly selected by the owner, resolved again before delivery.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct SkillRef {
    pub name: String,
    pub path: String,
}

/// What to launch.
#[derive(Clone, Debug, Serialize, Deserialize, TS, Default)]
#[serde(default)]
pub struct LaunchSpec {
    pub mentions: Vec<PromptMention>,
    pub skills: Vec<SkillRef>,
    pub permission_mode: PermissionMode,
    /// Permit additional paid usage after included subscription quotas are exhausted.
    pub allow_paid_usage: bool,
    pub harness: Option<HarnessKind>,
    /// A named profile of `harness` from `config.toml` (environment, defaults, billing).
    pub profile: Option<String>,
    /// Stop new turns once the harness-reported cumulative cost reaches this many USD.
    pub budget_usd: Option<f64>,
    pub target: Target,
    /// A new Launchpad-owned Git worktree for a project target. None: run in the project checkout.
    pub worktree: Option<WorktreeOptions>,
    pub text: String,
    /// Uploaded images and files, retained with drafts and scheduled launches.
    pub attachments: Vec<FileAttachment>,
    pub presets: Vec<String>,
    pub context_projects: Vec<ContextProject>,
    pub github_refs: Vec<GitHubRef>,
    /// Files and line ranges added from the code viewer.
    pub files: Vec<FileRef>,
    /// Stored trace through a fixed sequence number, selected as launch context.
    pub agent_traces: Vec<AgentTraceRef>,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// The workspace of the window that launched it. A new scratch folder joins it; other targets keep their own.
    pub workspace_id: Option<String>,
    /// Tools of Home's registry (IDs, `mcp:<server>`) switched off for this launch, over the default tools
    /// (`tool_defaults`). Only tools granted to the workspace can be on.
    pub tools_off: Vec<String>,
    /// Granted tools switched on for this launch although the defaults leave them off.
    #[ts(optional, as = "Option<Vec<String>>")]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools_on: Vec<String>,
    /// The runner chosen for this launch (for example a Local VM for Docker). None: Home picks one.
    pub runner_id: Option<String>,
    /// A session of a bot's thread: Home composes it from the bot's files (`bots::message`).
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot: Option<BotLaunch>,
    /// A run of a routine (`crons.rs`): its prompt says what started it and how to end.
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routine: Option<RoutineLaunch>,
    /// Who sends its first message when it is not the owner (`MessageContext::from`), stamped by Home.
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<MessageFrom>,
}

impl LaunchSpec {
    /// The owner's own launch (not an agent's, a bot's or a routine run): only it may end in a pull request on
    /// GitHub (`WhenDone::publishes`), since that choice is the owner's click.
    pub fn owner_launch(&self) -> bool {
        self.from.is_none() && self.bot.is_none() && self.routine.is_none()
    }
    /// The launch's When done, as it may run: other launches open a Local PR instead of a pull request.
    pub fn when_done(&self) -> WhenDone {
        let w = self.worktree.as_ref().map(|o| o.when_done).unwrap_or_default();
        if self.owner_launch() { w } else { w.without_github() }
    }
}

/// What a routine's run tells the agent: the routine, what started this run and the related items.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq, Default)]
#[serde(default)]
pub struct RoutineLaunch {
    pub id: String,
    pub title: String,
    /// For example "Scheduled" or "GitHub, review requested: owner/repo#12".
    pub cause: String,
    /// The items behind an event (GitHub threads, a finished session), one per line.
    pub context: String,
}

/// What a bot session needs to know about its bot, carried in the launch spec to the runner.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq, Default)]
#[serde(default)]
pub struct BotLaunch {
    pub id: String,
    pub name: String,
    /// Its slug (its Telegram command).
    pub slug: String,
    /// BOT.md, the owner's instructions. Cleared in session lists.
    pub instructions: String,
    /// The thread's previous session, referenced by its trace (never inlined).
    pub previous: Option<BotHandoff>,
    /// The version of BOT.md (`bots::version`) the session started from or last received in a message: when
    /// BOT.md changed since, Home adds the new one to the next message (`bots::with_new_instructions`).
    #[serde(skip_serializing_if = "String::is_empty")]
    #[ts(optional, as = "Option<String>")]
    pub version: String,
    /// Why this session of the thread started (`bots::rotation`: first, profile, folder, model, effort,
    /// permissions, archived; new: the owner's Start a new session, with `note`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, as = "Option<String>")]
    pub reason: Option<String>,
    /// The owner's own words for a new session they asked for (Start a new session on the bot page), shown in the
    /// chat's divider.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, as = "Option<String>")]
    pub note: Option<String>,
    /// The permission mode bot.toml gave the session at start: `bots::rotation` compares bot.toml with it, so a
    /// mode the owner set on the session itself (Agents) keeps the thread going. None for older sessions.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, as = "Option<PermissionMode>")]
    pub permission_mode: Option<PermissionMode>,
    /// `inbox_done = true` in bot.toml (and more than read access): it may run `launchpad inbox done`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[ts(optional, as = "Option<bool>")]
    pub inbox_done: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct BotHandoff {
    pub session_id: String,
    pub title: String,
    #[ts(type = "number")]
    pub through_seq: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    Project {
        project_id: String,
    },
    Predraft {
        project_id: String,
    },
    ExistingPredraft {
        predraft_id: String,
    },
    /// Continue in a saved session's folder with a new native conversation.
    SessionFolder {
        session_id: String,
    },
    /// A new temporary folder in `<data_dir>/tmp`.
    Temp,
    /// Last: also what records of removed kinds (the former browser sessions) read as.
    #[default]
    #[serde(other)]
    None,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct AgentTraceRef {
    pub session_id: String,
    #[ts(type = "number")]
    pub through_seq: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct ContextProject {
    pub project_id: String,
    /// None: the current checkout.
    pub branch: Option<String>,
    /// The agent may modify it: it gets its own task worktree unless the launch runs in the project checkout.
    #[serde(default)]
    pub editable: bool,
}

/// How a task worktree starts and ends (`crates/core/src/worktrees.rs`).
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq, Default)]
#[serde(default)]
pub struct WorktreeOptions {
    /// Branch to start from. None: the folder's default branch on Home.
    pub base: Option<String>,
    /// Branch the work goes into (merge target, Local PR and pull request base). None: the branch it starts from
    /// (`base`), or, for a launch in another agent's worktree, that worktree's own base branch.
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub into: Option<String>,
    /// What happens when the agent runs `launchpad work finish`.
    pub when_done: WhenDone,
    /// Continue this pull request: start from its head; Home pushes the agent's commits to its branch.
    /// `base` and `when_done` do not apply.
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pull_request: Option<PullRequestRef>,
}

/// How a task worktree ends when the agent is done.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WhenDone {
    /// The branch stays for the owner to review, merge, publish or discard.
    #[default]
    Keep,
    /// `work finish` requests a merge into the base branch.
    Merge,
    /// `work finish` turns the worktree into a Local PR.
    LocalPr,
    /// `work finish` opens a Local PR; Home publishes it as a draft pull request on GitHub once the turn ended
    /// Done and its work is on Home (`pr_when_done`). Only for the owner's own launches.
    DraftPr,
    /// Like `DraftPr`, as a pull request ready for review.
    OpenPr,
    /// Questions and audits: the agent changes nothing, and archiving it deletes an unchanged worktree and
    /// its branch. Chosen per launch, never a folder default.
    ReadOnly,
}

impl WhenDone {
    pub fn as_str(self) -> &'static str {
        match self { Self::Keep => "keep", Self::Merge => "merge", Self::LocalPr => "local_pr", Self::DraftPr => "draft_pr", Self::OpenPr => "open_pr", Self::ReadOnly => "read_only" }
    }
    pub fn parse(s: &str) -> Self {
        match s { "merge" => Self::Merge, "local_pr" => Self::LocalPr, "draft_pr" => Self::DraftPr, "open_pr" => Self::OpenPr, "read_only" => Self::ReadOnly, _ => Self::Keep }
    }
    /// `work finish` opens a Local PR (and Home may publish it).
    pub fn opens_local_pr(self) -> bool {
        matches!(self, Self::LocalPr | Self::DraftPr | Self::OpenPr)
    }
    /// Home publishes the Local PR to GitHub after the turn: `Some(draft)`.
    pub fn publishes(self) -> Option<bool> {
        match self { Self::DraftPr => Some(true), Self::OpenPr => Some(false), _ => None }
    }
    /// What an agent or a bot may run with: a pull request on GitHub needs the owner's own launch, so it becomes a
    /// Local PR the owner publishes.
    pub fn without_github(self) -> Self {
        if self.publishes().is_some() { Self::LocalPr } else { self }
    }
}

/// A GitHub pull request, by repository and number.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct PullRequestRef {
    /// `owner/repo` of the base repository.
    pub repo: String,
    pub number: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct GitHubRef {
    pub url: String,
    /// `issue`, `pr`, `comment`, `review_comment` or another notification subject kind.
    pub kind: String,
    pub repo: String,
    pub number: Option<u64>,
    pub title: Option<String>,
}

/// A file from the code viewer, given to the agent as a reference: it reads the file itself.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct FileRef {
    /// The working tree, or the project checkout for a revision.
    pub root: String,
    /// None: the file on disk in `root`.
    pub rev: Option<String>,
    pub path: String,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: String,
    pub remote_url: Option<String>,
    /// `owner/repo` of `origin`.
    pub gh_repo: Option<String>,
    /// `owner/repo` of `upstream`, for forks.
    pub gh_upstream: Option<String>,
    pub description: String,
    pub is_temp: bool,
    pub pinned: bool,
    #[ts(type = "number")]
    pub created_at: i64,
    /// Owner-configured command run in each new task worktree before the agent starts. Empty: none.
    pub worktree_setup: String,
    /// Default task worktree completion choice, remembered when selected in the composer.
    #[serde(default)]
    pub when_done: WhenDone,
    /// The one workspace of this folder. None: Unassigned (folders registered before workspaces were required).
    #[serde(default)]
    pub workspace_id: Option<String>,
    /// On Home: the runner that has this temporary folder now. None for every other folder: Home stores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub runner_id: Option<String>,
    /// A Git folder served by Home Git: Home owns the record and the branches; runners clone it on first use.
    #[serde(default)]
    pub home_git: bool,
    /// A plain folder (no Git) stored by Home, with previous versions and trash; runners keep a synced copy.
    #[serde(default)]
    pub plain: bool,
    /// The bot the extension's Ask uses on this folder's GitHub pages (a bot of its workspace). None: the last bot used
    /// on the repository, else the workspace's Ask bot. Home's record only (runners' copies leave it out).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub bot: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ModelInfo {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub efforts: Vec<String>,
    /// Inference providers serving the model (Hugging Face router lists), selected with an
    /// `id:provider` suffix. Empty for harness model lists.
    #[serde(default)]
    pub providers: Vec<ModelProvider>,
}

/// One provider of a router model, with its reported prices in USD per million tokens.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ModelProvider {
    pub name: String,
    #[ts(type = "number | null")]
    pub context_length: Option<u64>,
    pub input_usd: Option<f64>,
    /// Cached input (cache read), when the provider reports it.
    pub cached_input_usd: Option<f64>,
    pub output_usd: Option<f64>,
    /// Output tokens per second, as measured by the router.
    pub throughput: Option<f64>,
    pub first_token_ms: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct HarnessDefaults {
    pub model: Option<String>,
    pub effort: Option<String>,
    pub models: Vec<ModelInfo>,
    /// Effort levels when the model is not known.
    pub efforts: Vec<String>,
}

/// A durable pending continuation. Claim it before sending; uncertain claims need user action.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct QuotaPause {
    pub phase: QuotaPausePhase,
    pub blockers: Vec<QuotaBlock>,
    pub message: String,
    pub images: Vec<ImageAttachment>,
    /// Existing queued input, or None for an interrupted turn's continuation message.
    pub client_id: Option<String>,
    pub error: Option<String>,
    /// Owner steering, delivered with the continuation in submission order.
    #[serde(default)]
    pub messages: Vec<QuotaMessage>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct QuotaMessage {
    pub text: String,
    pub images: Vec<ImageAttachment>,
    pub client_id: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QuotaPausePhase { Pausing, Paused, Resuming, NeedsAttention, ProviderLimited }

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct QuotaBlock {
    pub window: String,
    #[ts(type = "number | null")]
    pub resets_at: Option<i64>,
}

#[cfg(test)]
mod when_done_tests {
    use super::*;

    #[test]
    fn pull_requests_only_for_the_owners_launches() {
        let mut spec = LaunchSpec { worktree: Some(WorktreeOptions { when_done: WhenDone::DraftPr, ..Default::default() }), ..Default::default() };
        assert_eq!(spec.when_done(), WhenDone::DraftPr);
        assert_eq!(spec.when_done().publishes(), Some(true));
        spec.routine = Some(RoutineLaunch::default());
        assert_eq!(spec.when_done(), WhenDone::LocalPr);
        spec.routine = None;
        spec.worktree.as_mut().unwrap().when_done = WhenDone::Merge;
        spec.from = Some(Default::default());
        assert_eq!(spec.when_done(), WhenDone::Merge);
        for w in [WhenDone::Keep, WhenDone::Merge, WhenDone::LocalPr, WhenDone::DraftPr, WhenDone::OpenPr, WhenDone::ReadOnly] {
            assert_eq!(WhenDone::parse(w.as_str()), w);
        }
    }
}

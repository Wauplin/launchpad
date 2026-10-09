//! Types shared by the session manager, the harness adapters, the API and the UI (exported with ts-rs).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum HarnessKind {
    Claude,
    Codex,
}

impl HarnessKind {
    pub fn as_str(self) -> &'static str {
        match self {
            HarnessKind::Claude => "claude",
            HarnessKind::Codex => "codex",
        }
    }
    pub fn display(self) -> &'static str {
        match self {
            HarnessKind::Claude => "Claude",
            HarnessKind::Codex => "Codex",
        }
    }
    pub fn parse(s: &str) -> Option<HarnessKind> {
        match s {
            "claude" => Some(HarnessKind::Claude),
            "codex" => Some(HarnessKind::Codex),
            _ => None,
        }
    }
}

/// Session states, the same for every harness (ARCHITECTURE.md, section 9.1).
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
    /// Claude only (`total_cost_usd`, cumulative for the session).
    pub cost_usd: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Session {
    pub id: String,
    pub harness: HarnessKind,
    /// Claude session id (same as `id`) or Codex thread id.
    pub harness_session_id: Option<String>,
    pub project_id: Option<String>,
    pub cwd: String,
    pub title: String,
    pub status: SessionStatus,
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
    /// Sent by you. `queued` while the agent has not read it yet.
    UserMessage { text: String, client_id: String, queued: bool, #[serde(default)] images: Vec<ImageAttachment> },
    /// The agent read a queued message.
    MessageDelivered { client_id: String },
    /// Live text of an assistant message (not stored; the final text comes as `assistant_message`).
    AssistantDelta { item_id: String, text: String },
    AssistantMessage { item_id: String, text: String },
    Reasoning { item_id: String, text: String },
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
        matches!(self, EventBody::AssistantDelta { .. } | EventBody::ToolOutput { .. })
    }
    pub fn kind(&self) -> &'static str {
        match self {
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

/// What to launch (ARCHITECTURE.md, section 9.7).
#[derive(Clone, Debug, Serialize, Deserialize, TS, Default)]
#[serde(default)]
pub struct LaunchSpec {
    pub harness: Option<HarnessKind>,
    pub target: Target,
    pub text: String,
    pub presets: Vec<String>,
    pub context_projects: Vec<ContextProject>,
    pub github_refs: Vec<GitHubRef>,
    /// Files and line ranges added from the code viewer.
    pub files: Vec<FileRef>,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// The workspace of the window that launched it: the target project joins that workspace.
    pub workspace_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    #[default]
    None,
    Project {
        project_id: String,
    },
    Predraft {
        project_id: String,
    },
    ExistingPredraft {
        predraft_id: String,
    },
    /// A new temporary folder in the projects root.
    Temp,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
pub struct ContextProject {
    pub project_id: String,
    /// None: the current checkout.
    pub branch: Option<String>,
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
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ModelInfo {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub efforts: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct HarnessDefaults {
    pub model: Option<String>,
    pub effort: Option<String>,
    pub models: Vec<ModelInfo>,
    /// Effort levels when the model is not known.
    pub efforts: Vec<String>,
}

//! The action list: every action that a button, a card, the Launchpad CLI or the admin rules name, defined once
//! with its API call, its input, its confirmation, its effect and who may run it. The handlers stay in their
//! crates; `agent_auth::admin_allows` (http), `launchpad_cli::cli_reference`, `launchpad do` (cli) and the UI's
//! `ActionButton` (through `gen-types`) all read this list.
use serde::Serialize;
use ts_rs::TS;

/// What an action does, which decides who may run it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ActionEffect {
    /// Checks its input and changes nothing.
    Checks,
    /// Changes a Launchpad record (rename, pause, archive, edit a routine).
    Record,
    /// Starts agent work (launch, message, run a routine now).
    Starts,
    /// Reaches GitHub or other people (merge, publish, post, send, run CI): the owner's click only.
    Outside,
    /// Money, credentials or the machine: the owner's click only.
    Machine,
    /// Installs a newer Launchpad on Home's Space, or Home's build on a runner (it restarts; agents continue). Admin
    /// may run it when the owner asks; it has a confirmation and no undo.
    Installs,
}

/// Who may run an action directly. Sessions with admin Launchpad CLI access run `Admin` actions; `Owner`
/// actions need a click in an owner window (or wait as a card).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ActionAccess {
    Owner,
    Admin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "ActionFieldKind")]
pub enum FieldKind {
    Text,
    Bool,
    /// Any JSON value (lists, objects).
    Json,
}

/// One field of an action's JSON body.
#[derive(Clone, Copy, Debug, Serialize, TS)]
#[ts(rename = "ActionField")]
pub struct Field {
    pub name: &'static str,
    pub kind: FieldKind,
    pub optional: bool,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
pub struct Action {
    /// `<group>.<verb>`: `launchpad do <name>` and the short verb `launchpad <group> <verb>`.
    pub name: &'static str,
    pub method: &'static str,
    /// The API path, with `{param}` segments filled in order from the positional arguments.
    pub route: &'static str,
    /// Button label.
    pub label: &'static str,
    /// Sprite icon name (`web/src/components/Icon.tsx`).
    pub icon: Option<&'static str>,
    /// One line for the CLI reference and button titles.
    pub help: &'static str,
    /// Body fields, from `--<name> <value>` options.
    pub input: &'static [Field],
    /// The whole body comes from `--body-file` (what it holds).
    pub body_file: Option<&'static str>,
    /// Fixed body fields (a JSON object) merged into the input, such as `{"action": "done"}`.
    pub fixed: Option<&'static str>,
    /// What the second click does. `{name}` takes a route parameter or a value the button passes.
    pub confirm: Option<&'static str>,
    pub effect: ActionEffect,
    pub access: ActionAccess,
    /// The proposal card kind whose acceptance runs this action.
    pub card: Option<&'static str>,
    /// The action that undoes this one, when there is one (an action with an undo needs no confirmation).
    pub undo: Option<&'static str>,
    /// For an undo: the GET route (same parameters) whose answer holds the values before the change, by body key.
    /// The undo then runs `undo` with the same parameters and those values. None: the action creates a record and
    /// the undo takes the new record's `id` as its only parameter.
    pub before: Option<&'static str>,
    /// An agent may propose it as a ready card: the owner's click runs it, with the same button and confirmation.
    pub ready: bool,
}

const fn f(name: &'static str, kind: FieldKind) -> Field {
    Field { name, kind, optional: false }
}
const fn opt(name: &'static str, kind: FieldKind) -> Field {
    Field { name, kind, optional: true }
}

use ActionAccess::{Admin, Owner};
use ActionEffect::{Checks, Installs, Machine, Outside, Record, Starts};
use FieldKind::{Bool, Json, Text};

const A: Action = Action { name: "", method: "POST", route: "", label: "", icon: None, help: "", input: &[], body_file: None, fixed: None, confirm: None, effect: Record, access: Admin, card: None, undo: None, before: None, ready: false };

/// Every action. Keep the names stable: the UI and agents use them.
pub const ACTIONS: &[Action] = &[
    // Topics are workspace collections; membership changes never start agent work.
    Action { name: "topic.create", route: "/api/topics", label: "Create topic", help: "Create a workspace topic", input: &[f("workspace_id", Text), f("name", Text)], access: Owner, undo: Some("topic.delete"), ..A },
    Action { name: "topic.rename", method: "PATCH", route: "/api/topics/{id}", label: "Rename", help: "Rename a topic", input: &[f("name", Text)], access: Owner, undo: Some("topic.rename"), before: Some("/api/topics/{id}"), ..A },
    Action { name: "topic.delete", method: "DELETE", route: "/api/topics/{id}", label: "Delete topic", help: "Delete a topic; its sessions stay", access: Owner, confirm: Some("Deletes this topic. Its sessions and previously attached context stay."), ..A },
    Action { name: "topic.members", route: "/api/topics/{id}/members", label: "Update topics", help: "Add or remove session memberships", input: &[f("session_ids", Json), f("present", Bool)], access: Owner, ..A },
    // Sessions.
    Action { name: "session.launch", route: "/api/sessions", label: "Launch", icon: Some("play"), help: "Launch an agent", body_file: Some("a launch spec"), effect: Starts, card: Some("launch"), ..A },
    Action { name: "session.message", route: "/api/sessions/{id}/input", label: "Send", icon: Some("send"), help: "Send a message to an agent session", input: &[f("text", Text)], effect: Starts, card: Some("message"), ..A },
    Action { name: "session.answer", route: "/api/sessions/{id}/answer", label: "Answer", help: "Answer an agent's open question (request_id from `sessions show`); a permission request (Allow or Deny) is the owner's", input: &[f("request_id", Text), f("answers", Json)], effect: Starts, ..A },
    Action { name: "session.stop", route: "/api/sessions/{id}/interrupt", label: "Stop", icon: Some("stop"), help: "Stop an agent's turn", ..A },
    Action { name: "session.archive", route: "/api/sessions/{id}/close", label: "Archive", icon: Some("archive"), help: "Archive an agent session (stops it first)", ..A },
    Action { name: "session.archive-done", route: "/api/workspaces/{workspace}/archive-done", label: "Archive", icon: Some("archive"), help: "Archive automation threads: archive the finished automation threads of a workspace that the preview (`GET /api/context/archive-done?workspace=…&bot=…&older_than=…`, `launchpad sessions archive-done`) listed: routine runs, bot threads started by a routine, channel, DM, hook or another agent, and the threads they handed off to; only those still Done with nothing waiting (no question, card, Suggestion, Local PR, follow-up, and the owner never took part); a conversation goes whole; traces stay and Unarchive brings each back (session_ids: the preview's; bot and older_than: the preview's filter)", input: &[f("session_ids", Json), opt("bot", Text), opt("older_than", Text)], confirm: Some("Archives the threads the preview listed. Their history stays in Agents and Unarchive brings each one back."), ..A },
    Action { name: "session.unarchive", route: "/api/sessions/{id}/unarchive", label: "Unarchive", icon: Some("archive"), help: "Bring back an archived agent (Done, or Stopped if it was not Done); its next message resumes it", ..A },
    Action { name: "session.resume", route: "/api/sessions/{id}/resume", label: "Resume", icon: Some("play"), help: "Resume an interrupted or failed agent", input: &[opt("continue_turn", Bool)], effect: Starts, ..A },
    Action { name: "session.resume-subscription", route: "/api/sessions/{id}/resume-subscription", label: "Resume on subscription", icon: Some("play"), help: "Resume a quota-paused agent on its subscription", effect: Starts, ..A },
    Action { name: "session.rename", route: "/api/sessions/{id}/rename", label: "Rename", icon: Some("pencil"), help: "Rename an agent session", input: &[f("title", Text)], undo: Some("session.rename"), before: Some("/api/sessions/{id}"), ..A },
    Action { name: "session.auto-title", route: "/api/sessions/{id}/auto-title", label: "Auto", icon: Some("pencil"), help: "Rename an agent session with a title generated now from its conversation (on demand; a rename made meanwhile wins)", undo: Some("session.rename"), before: Some("/api/sessions/{id}"), ..A },
    Action { name: "session.model", route: "/api/sessions/{id}/model", label: "Change model", help: "Change an agent's model or effort for its next turns", input: &[opt("model", Text), opt("effort", Text)], ..A },
    Action { name: "session.permissions", route: "/api/sessions/{id}/permissions", label: "Change permissions", help: "Change an agent's permission mode (yolo, approvals or auto); an agent may only make it stricter", input: &[f("mode", Text)], ..A },
    Action { name: "session.follow-up", route: "/api/sessions/{id}/follow-up", label: "Follow up", icon: Some("clock"), help: "Schedule a follow-up message to an agent", input: &[f("message", Text), f("conditions", Json)], effect: Starts, ..A },
    Action { name: "session.seen", route: "/api/sessions/{id}/seen", label: "Mark seen", icon: Some("eye"), help: "Mark an agent as seen", ..A },
    Action { name: "session.unread", route: "/api/sessions/{id}/unread", label: "Mark unread", icon: Some("eye"), help: "Mark an agent as unread", ..A },
    Action { name: "session.resume-paid", route: "/api/sessions/{id}/resume-paid", label: "Resume with paid usage", help: "Resume a quota-paused agent with paid usage", confirm: Some("Continues on paid usage, billed to the profile's account."), effect: Machine, access: Owner, ..A },
    // Bots.
    Action { name: "bot.create", route: "/api/bots", label: "Create bot", icon: Some("plus"), help: "Create a bot (it starts nothing; an agent's bot gets at most propose access)", body_file: Some("{\"workspace_id\", \"name\", \"instructions\", \"profile\", \"folder\"}"), card: Some("bot"), undo: Some("bot.delete"), ..A },
    Action { name: "bot.update", method: "PATCH", route: "/api/bots/{id}", label: "Save", help: "Change a bot's settings, its BOT.md or its routine files (an agent never raises `launchpad` or loosens `permission_mode`)", body_file: Some("{\"set\": {<bot.toml fields to change, for example \"proposals\": \"manual\"; null puts back the default>}}, {\"settings\": <all of bot.toml>}, {\"instructions\": \"<whole BOT.md>\"} or {\"files\": {\"routines/<name>.toml\": \"<whole file>\" or null to delete it}}"), card: Some("bot_files"), undo: Some("bot.update"), before: Some("/api/bots/{id}"), ..A },
    Action { name: "bot.delete", method: "DELETE", route: "/api/bots/{id}", label: "Delete bot", icon: Some("trash"), help: "Delete a bot, its memory and its routines", confirm: Some("Deletes the bot, its memory and its routines. Its sessions stay in Agents."), ..A },
    Action { name: "bot.message", route: "/api/bots/{id}/message", label: "Send", icon: Some("send"), help: "Send a message to a bot (the owner's: to `thread`, else a new thread (also with new_thread), or the thread of the one GitHub item of the context with item_thread, or the bot's thread of a conversation with conversation (a bot that is not part of it joins it); an agent's: its own thread of the bot, or from a bot's thread its thread of the sender's conversation, waiting for a free slot when the bot works in max_threads threads; a thread whose session cannot go on, archived or after a change of the bot's profile, folder, model, effort or permission mode, continues in a new thread, which the returned session names; the owner's message is stored on Home and sent in the background: with accept true the answer is its receipt at once (id, thread, state), else the session once sent); context (optional) adds read-only folders and agent traces: {\"context_projects\": [{\"project_id\": \"…\", \"branch\": null}], \"agent_traces\": [{\"session_id\": \"…\", \"through_seq\": 12}]}; in (45m, 2h, 1d) or at (RFC 3339 time, such as 2026-11-21T09:00:00Z) sends it later instead: Home stores it (across restarts) and, once due, delivers it like the same message sent then (same sender, thread or conversation and loop guard; it waits for the end of a running turn, confirmed quota and a free slot); the answer names its n, which bot.cancel-queued cancels, and `launchpad bot scheduled` lists those waiting", input: &[f("text", Text), opt("context", Json), opt("thread", Text), opt("new_thread", Bool), opt("item_thread", Bool), opt("conversation", Text), opt("in", Text), opt("at", Text)], effect: Starts, ..A },
    Action { name: "bot.retry-send", route: "/api/bots/{id}/sends/{send}/retry", label: "Retry", icon: Some("refresh"), help: "Send again an owner's bot message that failed or whose send is uncertain (same ID: a message the bot already received is not delivered twice)", effect: Starts, access: Owner, ..A },
    Action { name: "bot.remove-send", method: "DELETE", route: "/api/bots/{id}/sends/{send}", label: "Remove", icon: Some("x"), help: "Remove an owner's bot message that was not sent (waiting, failed or uncertain); it is never sent", confirm: Some("Removes your message: it is never sent."), access: Owner, ..A },
    Action { name: "bot.thread-rename", method: "PATCH", route: "/api/bots/{id}/threads/{thread}", label: "Rename", icon: Some("pencil"), help: "Rename one of a bot's threads (threads from GET /api/bots/{id}/threads)", input: &[f("title", Text)], ..A },
    Action { name: "bot.thread-auto-title", route: "/api/bots/{id}/threads/{thread}/auto-title", label: "Auto", icon: Some("pencil"), help: "Rename one of a bot's threads with a title generated now from its conversation (on demand; a rename made meanwhile wins)", ..A },
    Action { name: "bot.thread-archive", route: "/api/bots/{id}/threads/{thread}/archive", label: "Archive thread", icon: Some("archive"), help: "Archive one of a bot's threads (its session is archived; archived false brings both back)", input: &[opt("archived", Bool)], confirm: Some("Archives this thread and its open session. Its history stays in Agents; a new message to its source starts a new thread."), ..A },
    // Chats: a pinned conversation of bots or channel is a shortcut in the laptop sidebar (and first on the phone).
    Action { name: "chat.pin", route: "/api/chat-pins", label: "Pin", icon: Some("pin"), help: "Pin or unpin a chat (target `conversation:<id>` or `channel:<id>`) as a shortcut in the sidebar", input: &[f("workspace_id", Text), f("target", Text), f("pinned", Bool)], access: Owner, ..A },
    Action { name: "bot.pause", route: "/api/bots/{id}/pause", label: "Pause", help: "Pause or resume a bot (and its routines and hooks)", input: &[f("paused", Bool)], undo: Some("bot.pause"), before: Some("/api/bots/{id}"), ..A },
    Action { name: "bot.cancel-queued", method: "DELETE", route: "/api/bots/{id}/queued/{n}", label: "Cancel", icon: Some("x"), help: "Cancel a message that waits for a bot: a hook, channel or agent message waiting for a busy bot, or a scheduled message (n from GET /api/bots/{id}/queued or `launchpad bot scheduled`); it is never sent", confirm: Some("Deletes the waiting message: it is never sent."), ..A },
    Action { name: "bot.restore", route: "/api/bots/{id}/restore/{change}", label: "Restore", icon: Some("refresh"), help: "Restore an earlier version of a bot's files", ..A },
    Action { name: "bot.add-routine", route: "/api/bots/{id}/routines", label: "Add routine", icon: Some("plus"), help: "Add a routine to a bot", body_file: Some("{\"title\", \"instructions\", \"trigger\", \"schedule\", \"conditions\", \"auto_archive\"}"), card: Some("routine"), undo: Some("routine.delete"), ..A },
    // Routines and scheduled launches.
    Action { name: "routine.create", route: "/api/launch/crons", label: "Create routine", icon: Some("clock"), help: "Create a routine that launches a spec", body_file: Some("{\"spec\": <launch spec>, \"schedule\", \"conditions\", \"trigger\", \"title\", \"auto_archive\"}"), card: Some("cron"), undo: Some("routine.delete"), ..A },
    Action { name: "routine.update", method: "PATCH", route: "/api/launch/crons/{id}", label: "Save", help: "Change a routine (schedule, conditions, paused, auto_archive, title, instructions)", body_file: Some("the fields to change"), undo: Some("routine.update"), before: Some("/api/launch/crons/{id}"), ..A },
    Action { name: "routine.delete", method: "DELETE", route: "/api/launch/crons/{id}", label: "Delete", icon: Some("trash"), help: "Delete a routine", confirm: Some("Deletes the routine. Its past runs stay in Agents."), ..A },
    Action { name: "routine.run", route: "/api/launch/crons/{id}/run", label: "Run now", icon: Some("play"), help: "Run a routine now", effect: Starts, ..A },
    // Oracles (`launchpad_automation::oracles`): Python files of a workspace's store that Home runs in Monty. Saving
    // is admin for a file whose credentialed tools the session holds (another file is an oracle card).
    // Recall: a derived search index over each workspace's past sessions (`launchpad_recall`). Reads (`launchpad recall`) are not actions.
    Action { name: "recall.set", method: "PATCH", route: "/api/recall/config", label: "Save", icon: Some("preset"), help: "Change Home's recall settings (enabled: Home indexes finished turns and answers `launchpad recall`; include_archived: also index archived sessions; hidden: the whole list of session IDs never returned)", input: &[opt("enabled", Bool), opt("include_archived", Bool), opt("hidden", Json)], undo: Some("recall.set"), before: Some("/api/recall/config"), ..A },
    Action { name: "recall.backfill", route: "/api/recall/{workspace}/backfill", label: "Backfill", icon: Some("refresh"), help: "Index every stored session of a workspace for recall, in the background on Home: skips what is already indexed, continues after an interruption, and keeps the workspace's whole history indexed from then on (dry_run: only count what it would do); `launchpad recall status` shows its progress and what it left out", input: &[opt("dry_run", Bool)], ..A },
    Action { name: "recall.rebuild", route: "/api/recall/{workspace}/rebuild", label: "Rebuild", icon: Some("refresh"), help: "Delete a workspace's recall memory and index its history again from the stored sessions: the only way the memory forgets (archived and hidden sessions, a converter or model change)", confirm: Some("Deletes this workspace's recall memory and builds it again from the stored sessions. Recall misses history until it is done."), ..A },
    Action { name: "oracle.save", method: "PUT", route: "/api/oracles/{workspace}/{slug}", label: "Save", icon: Some("pencil"), help: "Create an oracle or change its file (content: the whole `oracles/<slug>.py`); pre-checked and stored formatted; an agent saves only a file whose credentialed tools (github, huggingface) its own session holds: propose an oracle card for another", input: &[f("content", Text)], card: Some("oracle"), undo: Some("oracle.save"), before: Some("/api/oracles/{workspace}/{slug}"), ..A },
    Action { name: "oracle.pause", route: "/api/oracles/{workspace}/{slug}/pause", label: "Pause oracle", icon: Some("stop"), help: "Pause an oracle: nothing evaluates it, and routines gated on it record quiet checks", ..A },
    Action { name: "oracle.resume", route: "/api/oracles/{workspace}/{slug}/resume", label: "Resume", icon: Some("play"), help: "Resume a paused oracle", ..A },
    Action { name: "oracle.run", route: "/api/oracles/{workspace}/{slug}/run", label: "Run", icon: Some("play"), help: "Run a saved oracle on Home and get its answer (params: {name: value}; cursor: a named cursor, committed when it answers True or False; since: a cursor value for one stateless run; max_age: accept a result this fresh, such as 2m); every Launchpad CLI access level may run it", input: &[opt("params", Json), opt("cursor", Text), opt("since", Text), opt("max_age", Text)], effect: Checks, ..A },
    Action { name: "oracle.test", route: "/api/oracles/{workspace}/{slug}/test", label: "Test now", icon: Some("play"), help: "Evaluate an oracle on Home now, recorded as a test that commits no cursor (content: a draft, pre-checked, then run like the saved oracle and not recorded; params, else those of its last evaluation; cursor)", input: &[opt("params", Json), opt("content", Text), opt("cursor", Text)], effect: Checks, access: Owner, ..A },
    Action { name: "oracle.mark-read", route: "/api/oracles/{workspace}/{slug}/read", label: "Mark read", icon: Some("check"), help: "Hide an oracle's failure row in Today until its next failure", access: Owner, ..A },
    Action { name: "oracle.restore", route: "/api/oracles/{workspace}/{slug}/restore/{change}", label: "Restore", icon: Some("refresh"), help: "Restore an oracle's file as it was before a change (change: from its Versions)", access: Owner, ..A },
    Action { name: "oracle.delete", method: "DELETE", route: "/api/oracles/{workspace}/{slug}", label: "Delete oracle", icon: Some("trash"), help: "Delete an oracle with its evaluations and cursors (its file stays restorable 30 days); force: also while routines are gated on it", input: &[opt("force", Bool)], confirm: Some("Deletes this oracle, its evaluations and its cursors. Routines gated on it fail until you change their gate."), access: Owner, ..A },
    // Folios: agents create, write, fix, reply and resolve as their own writes (propose access, `launchpad folio …`);
    // marks, Send feedback, Restore, Archive and Delete are the owner's.
    Action { name: "folio.create", route: "/api/folios/{workspace}", label: "New folio", icon: Some("plus"), help: "Create a folio from its kind's skeleton (title; kind: report, roadmap, audit, brief, status or sketchbook; slug); agents use `launchpad folio create` at propose access", input: &[f("title", Text), opt("kind", Text), opt("slug", Text)], ..A },
    Action { name: "folio.write", route: "/api/folios/{workspace}/{slug}/write", label: "Write", icon: Some("pencil"), help: "Write files of a folio (ops: write, append, patch, remove; base_rev: the rev you read; strict: refuse a write with errors); checked, and its problems listed in the answer; agents use `launchpad folio write|append|patch|add-media|rm`", input: &[f("ops", Json), opt("base_rev", Json), opt("strict", Bool)], ..A },
    Action { name: "folio.fix", route: "/api/folios/{workspace}/{slug}/fix", label: "Apply fixes", icon: Some("check"), help: "Apply the check's safe fixes of a folio as one rev labeled check --fix", ..A },
    Action { name: "folio.done", route: "/api/folios/{workspace}/{slug}/done", label: "Done", icon: Some("check"), help: "An agent closes its round of changes to a folio: its revs since its last done become one rev (message: its label), so the folio keeps one rev per round instead of one per write; agents use `launchpad folio done`", input: &[opt("message", Text)], ..A },
    Action { name: "folio.reply", route: "/api/folios/{workspace}/{slug}/marks/{id}/reply", label: "Reply", icon: Some("comment"), help: "Reply under a comment of a folio", input: &[f("text", Text)], ..A },
    Action { name: "folio.resolve", route: "/api/folios/{workspace}/{slug}/marks/{id}/resolve", label: "Resolve", icon: Some("check"), help: "Resolve a comment of a folio, naming the rev that addressed it (rev, default the latest; open: true opens it again)", input: &[opt("rev", Json), opt("open", Bool)], ..A },
    Action { name: "folio.mark", route: "/api/folios/{workspace}/{slug}/marks", label: "Save", icon: Some("comment"), help: "Save your mark on a folio: a comment (text, page, target, quote), a verdict (target: an item id; value: do, later, wont or discuss), a choice (target: a decision id; value: an option) or a tick (value: true or false); saved at once, sent with Send feedback", input: &[f("kind", Text), opt("page", Text), opt("target", Text), opt("quote", Text), opt("prefix", Text), opt("suffix", Text), opt("value", Text), opt("text", Text), opt("id", Json)], access: Owner, ..A },
    Action { name: "folio.unmark", method: "DELETE", route: "/api/folios/{workspace}/{slug}/marks/{id}", label: "Delete", icon: Some("trash"), help: "Delete your comment on a folio", confirm: Some("Deletes this comment."), access: Owner, ..A },
    Action { name: "folio.send-feedback", route: "/api/folios/{workspace}/{slug}/send", label: "Send feedback", icon: Some("send"), help: "Send your unsent marks on a folio in one short message (kind: thread, the folio's bot thread or bot_id + thread_id; session: session_id; new_thread: a new thread of bot_id with the folio's topic as context; note: a line of your own)", input: &[f("kind", Text), opt("bot_id", Text), opt("thread_id", Text), opt("session_id", Text), opt("note", Text)], effect: Starts, access: Owner, ..A },
    Action { name: "folio.ask-fix", route: "/api/folios/{workspace}/{slug}/ask-fix", label: "Ask the author to fix", icon: Some("send"), help: "Send one line asking a folio's agent to fix its problems (kind: thread, session or new_thread, like Send feedback); your marks stay unsent", input: &[f("kind", Text), opt("bot_id", Text), opt("thread_id", Text), opt("session_id", Text), opt("note", Text)], effect: Starts, access: Owner, ..A },
    Action { name: "folio.ask", route: "/api/folios/{workspace}/{slug}/ask", label: "Ask", icon: Some("send"), help: "Ask the folio's agent about it: your question goes to the folio's thread (kind: thread, session or new_thread, like Send feedback), with the folio, its page, section and selected quote as context (a reference, never the text); your marks stay unsent", input: &[f("text", Text), opt("page", Text), opt("section", Text), opt("quote", Text), opt("rev", Json), f("kind", Text), opt("bot_id", Text), opt("thread_id", Text), opt("session_id", Text)], effect: Starts, access: Owner, ..A },
    Action { name: "folio.data-set", method: "PUT", route: "/api/folios/{workspace}/{slug}/data/{key}", label: "Set", icon: Some("pencil"), help: "Set a key of a folio's data (JSON value; at most 64 KB per value, 1 MB and 2,000 keys per folio, last write wins); a sketch's data bridge writes only its own prefix; agents use `launchpad folio data set`", input: &[f("value", Json)], ..A },
    Action { name: "folio.data-delete", method: "DELETE", route: "/api/folios/{workspace}/{slug}/data/{key}", label: "Remove", icon: Some("trash"), help: "Remove a key of a folio's data; agents use `launchpad folio data rm`", confirm: Some("Removes this key and its value from the folio's data."), ..A },
    Action { name: "folio.render-problems", route: "/api/folios/{workspace}/{slug}/render", label: "Report render problems", icon: Some("alert"), help: "Save what the reader saw while rendering a page of a rev (F701 sketch errors and empty frames, F702 Mermaid and chart errors, F703 sideways scrolling on a phone): the next check lists them", input: &[f("rev", Json), f("page", Text), f("problems", Json)], access: Owner, ..A },
    Action { name: "folio.version", route: "/api/folios/{workspace}/{slug}/versions", label: "Name this version", icon: Some("tag"), help: "Name a rev of a folio (the latest by default) as a version kept for good (name; rev); naming it again renames it; agents use `launchpad folio version`", input: &[f("name", Text), opt("rev", Json)], ..A },
    Action { name: "folio.unversion", method: "DELETE", route: "/api/folios/{workspace}/{slug}/versions/{rev}", label: "Unpin this version", icon: Some("x"), help: "Make a named version an ordinary rev again: its files go after 30 days like the others", confirm: Some("Its files are kept 30 days from now, like other revs, then they go."), access: Owner, ..A },
    Action { name: "folio.share", route: "/api/folios/{workspace}/{slug}/share", label: "Publish snapshot", icon: Some("upload"), help: "Share a read-only snapshot of a rev of a folio as a static Space under your Hugging Face namespace (folio-<slug>; visibility: private or public, default private; days: expires after 30 by default; keep: no expiry; decisions: show your decisions as text; rev: default the latest); again updates it. No comments, no data API", input: &[opt("visibility", Text), opt("days", Json), opt("keep", Bool), opt("decisions", Bool), opt("rev", Json)], confirm: Some("Home publishes rev {rev} of this folio as a static Space on Hugging Face, readable by whoever the visibility allows."), effect: Outside, access: Owner, ready: true, ..A },
    Action { name: "folio.unshare", method: "DELETE", route: "/api/folios/{workspace}/{slug}/share", label: "Stop sharing", icon: Some("x"), help: "Delete a folio's shared snapshot (its Space) now", confirm: Some("Deletes the snapshot's Space on Hugging Face. The folio stays."), effect: Outside, access: Owner, ..A },
    Action { name: "folio.share-extend", route: "/api/folios/{workspace}/{slug}/share/extend", label: "Extend", icon: Some("clock"), help: "Extend a folio's shared snapshot (days from now, default 30) or keep it until Stop sharing (keep)", input: &[opt("days", Json), opt("keep", Bool)], access: Owner, ..A },
    Action { name: "folio.restore", route: "/api/folios/{workspace}/{slug}/revs/{rev}/restore", label: "Restore", icon: Some("refresh"), help: "Put a folio's files back as they were before a rev, as a new rev", confirm: Some("Puts the files of this rev back as they were before it, as a new rev."), access: Owner, ..A },
    Action { name: "folio.archive", route: "/api/folios/{workspace}/{slug}/archive", label: "Archive", icon: Some("archive"), help: "Archive a folio (archived: false brings it back with its files, revs and marks)", input: &[opt("archived", Bool)], access: Owner, ..A },
    Action { name: "folio.delete", method: "DELETE", route: "/api/folios/{workspace}/{slug}", label: "Delete folio", icon: Some("trash"), help: "Delete a folio with its files, revs and marks (its topic and sessions stay)", confirm: Some("Deletes this folio, its revs and your marks. Its topic and sessions stay."), access: Owner, ..A },
    Action { name: "schedule.create", route: "/api/launch/schedules", label: "Schedule", icon: Some("clock"), help: "Schedule a launch", body_file: Some("{\"spec\": <launch spec>, \"conditions\": […]}"), effect: Starts, card: Some("schedule"), undo: Some("schedule.cancel"), ..A },
    Action { name: "schedule.update", method: "PATCH", route: "/api/launch/schedules/{id}", label: "Save", icon: Some("pencil"), help: "Change a scheduled launch that has not started (its spec, and its conditions when given; an agent never loosens its permission mode)", body_file: Some("{\"spec\": <launch spec>, \"conditions\": […]}"), effect: Starts, ..A },
    Action { name: "schedule.cancel", method: "DELETE", route: "/api/launch/schedules/{id}", label: "Cancel", icon: Some("x"), help: "Cancel a scheduled launch", confirm: Some("Cancels this scheduled launch."), ..A },
    Action { name: "schedule.start", route: "/api/launch/schedules/{id}/start", label: "Start now", icon: Some("play"), help: "Start a scheduled launch now", effect: Starts, ..A },
    Action { name: "launch.check", route: "/api/launch/preview", label: "Check", help: "Check a launch spec without starting it", body_file: Some("a launch spec"), effect: Checks, ..A },
    Action { name: "launch.start-preview", route: "/api/launch/start-preview", label: "What it sees at start", help: "Show the instructions and first prompt a launch spec would give its agent, without starting it", body_file: Some("a launch spec"), effect: Checks, ..A },
    // Proposals.
    Action { name: "proposal.start", route: "/api/proposals/{id}/accept", label: "Start", icon: Some("play"), help: "Start a card (you may start a card only when you could run its action directly)", effect: Starts, ..A },
    Action { name: "proposal.dismiss", route: "/api/proposals/{id}/dismiss", label: "Dismiss", icon: Some("x"), help: "Dismiss a card (a report: mark it read)", ..A },
    Action { name: "proposal.reply", route: "/api/proposals/{id}/reply", label: "Reply", icon: Some("send"), help: "Answer a report: the text goes to the thread of the agent or bot that wrote it", input: &[f("text", Text)], effect: Starts, ..A },
    Action { name: "proposal.recheck", route: "/api/proposals/{id}/recheck", label: "Check again", icon: Some("refresh"), help: "Validate a card again", effect: Checks, ..A },
    // Runners.
    Action { name: "runner.takes-sessions", method: "PUT", route: "/api/runner/guard", label: "Takes new sessions", help: "In a runner's own window: whether Home starts new sessions there (paused: none; running sessions continue)", input: &[f("paused", Bool)], access: Owner, ..A },
    // A runner with another build than Home's updates itself to Home's build (`launchpad_core::self_update`). Admin
    // runs it when the owner asks, like a Home update.
    Action { name: "runner.update", route: "/api/runners/{id}/update", label: "Update", icon: Some("download"), help: "Update a connected runner that runs another build than Home (id from `runners list`, compat outdated) to Home's build: it downloads and checks Home's executable, waits until none of its agents is in a turn (yours too, when you run there) and restarts in place; its sessions continue after the restart. For a runner that is already on Home's build it is refused", confirm: Some("Installs Home's build on {name} and restarts it as soon as none of its agents is in a turn. Its open terminals close; its sessions continue after the restart."), effect: Installs, ..A },
    Action { name: "runner.cancel-update", method: "DELETE", route: "/api/runners/{id}/update", label: "Cancel", icon: Some("x"), help: "Cancel a runner update that still waits for its agents", confirm: Some("Cancels the update of {name}, which still waits for its agents."), ..A },
    // Exit routes (`egress`): which URLs leave through which runner. The owner's only: never admin, never in the
    // shared settings (`settings.set`).
    Action { name: "egress.set", method: "PUT", route: "/api/egress", label: "Save", help: "Set the exit routes (routes: the whole list of {name, github_org or urls, via}): URLs of a GitHub organization or URL prefixes that leave through one runner", input: &[f("routes", Json)], effect: Machine, access: Owner, undo: Some("egress.set"), before: Some("/api/egress"), ..A },
    Action { name: "runner.check-network", route: "/api/runners/{id}/network-check", label: "Check network", icon: Some("refresh"), help: "Check a runner's sandbox network: capture, DNS, UDP policy, IPv4 and IPv6, runner CA", effect: Checks, access: Owner, ..A },
    // Agent CLI updates on the runners (`launchpad_setup::cli_update`). Installing on a machine is the owner's
    // click; agents may check and mark results read.
    Action { name: "cli-update.check", route: "/api/cli-updates/check", label: "Check now", icon: Some("refresh"), help: "Read the newest agent CLI versions on npm now and update the runners (same major version) or ask for approval (new major version)", effect: Checks, ..A },
    Action { name: "cli-update.approve", route: "/api/cli-updates/{id}/approve", label: "Approve", icon: Some("download"), help: "Install a new major version of an agent CLI on the runners that wait for it (each one once no agent of it is in a turn)", confirm: Some("Installs {version} on the runners that wait for it, once no agent there is in a turn."), effect: Machine, access: Owner, ..A },
    Action { name: "cli-update.retry", route: "/api/cli-updates/{id}/retry", label: "Retry", icon: Some("refresh"), help: "Install an agent CLI update again on the runners where it did not install", confirm: Some("Installs {version} again on the runners where it did not install, once no agent there is in a turn."), effect: Machine, access: Owner, ..A },
    Action { name: "cli-update.dismiss", route: "/api/cli-updates/{id}/dismiss", label: "Mark read", icon: Some("check"), help: "Remove an agent CLI update from Today (a major version: skip it)", ..A },
    Action { name: "cli-update.auto", method: "PUT", route: "/api/cli-updates/auto", label: "Update automatically", help: "Turn automatic updates of one agent CLI on or off on every runner", input: &[f("harness", Text), f("on", Bool)], confirm: Some("Changes whether the runners update this CLI on their own."), effect: Machine, access: Owner, ..A },
    // Home updates (a Home on a Hugging Face Space: `launchpad_setup::home_update`). Admin runs them when the owner
    // asks, with `update_runners` when the owner also asked for the runners (as the button's Then update the runners).
    Action { name: "home.check-update", route: "/api/home/update/check", label: "Check for updates", icon: Some("refresh"), help: "Check whether a newer Launchpad exists for Home's Space (release or main, as deployed): deployed, latest and available", effect: Checks, ..A },
    Action { name: "home.update-cancel", route: "/api/home/update/cancel", label: "Cancel update", icon: None, help: "Put the previous version back on Home's Space while an update waits for its replacement: commits the Dockerfile from before the update on top of the Space's head, so the update can never land; Home accepts writes once the Space shows that version with no build in progress (it usually rebuilds and restarts from the checkpoint, losing nothing)", confirm: Some("Home's Space gets its previous version back and usually rebuilds and restarts with it. Changes stay paused until the Space shows it; nothing is lost."), access: Owner, ..A },
    Action { name: "home.update", route: "/api/home/update", label: "Update Home", icon: Some("download"), help: "Install a newer Launchpad on Home's Space: the newest of the deployed channel, or of `channel` (release or main); `version`, when given, must be that newest one (from home check-update); `update_runners` true: once the new Home runs, it updates each connected runner on another build to it (each once no agent there is in a turn, for 24 hours). The call returns once the Space starts its build, after Home paused writes (maintenance: every write answers 503 with `retryable` until the new Home runs) and saved a verified final backup; an agent that asks is woken once Home accepts work again (do not sleep yourself); a lost answer is safe to retry (it returns the update's state); Home restarts a few minutes later, while agents continue on their runners", input: &[opt("version", Text), opt("channel", Text), opt("update_runners", Bool)], confirm: Some("Home's Space installs {version}, rebuilds and restarts. Home pauses every change until the new version runs (several minutes): windows show it, agents and the CLI get a retryable error and running agents keep their work on their runners."), effect: Installs, ready: true, ..A },
    // Bot channels and DMs (`launchpad_channels::channel`). A post from an agent's token posts as its bot, only in a
    // channel it is a member of. Members, new channels and a channel's name and purpose are admin (recorded with Undo); the rest is the owner's.
    Action { name: "channel.post", route: "/api/channels/{id}/messages", label: "Send", icon: Some("send"), help: "Post the owner's own message in a bot channel or DM (bots post with `launchpad channel post` and `launchpad dm`); it wakes the bots it mentions and the members in mode all", input: &[f("text", Text), opt("reply_to", Json), opt("ask_owner", Bool)], effect: Starts, access: Owner, ..A },
    Action { name: "channel.dm", route: "/api/channels/dm", label: "Open DM", icon: Some("comment"), help: "Open the DM of two bots of one workspace (created on first use; both bots are members in mode all; a bot opens its own with `launchpad dm`)", input: &[f("bots", Json)], access: Owner, ..A },
    Action { name: "channel.create", route: "/api/channels", label: "New channel", icon: Some("plus"), help: "Create a named bot channel in your workspace (name: lowercase letters, digits and dashes; members: bot slugs or ids, comma-separated or a JSON list); its member bots are woken when mentioned (mode mentions) unless a member is {\"bot\": \"release\", \"mode\": \"all\"}; workspace_id: an agent's is its own, the owner names one", input: &[f("name", Text), opt("purpose", Text), opt("members", Json), opt("workspace_id", Text)], card: Some("channel"), ..A },
    Action { name: "channel.add-member", method: "PUT", route: "/api/channels/{id}/members/{bot}", label: "Add member", icon: Some("plus"), help: "Add a bot of the channel's workspace to a named channel, or change its mode: all (every message), mentions, digest or muted (null removes it)", input: &[f("mode", Text)], card: Some("channel_join"), undo: Some("channel.add-member"), before: Some("/api/channels/{id}/members/{bot}"), ..A },
    Action { name: "channel.set-mode", method: "PUT", route: "/api/channels/{id}/members/{bot}/mode", label: "Change mode", help: "Change when a member bot of a named channel is woken: all (every message), mentions, digest (a daily summary) or muted", input: &[f("mode", Text)], undo: Some("channel.set-mode"), before: Some("/api/channels/{id}/members/{bot}"), ..A },
    Action { name: "channel.remove-member", route: "/api/channels/{id}/members/{bot}/remove", label: "Remove", icon: Some("x"), help: "Remove a bot from a named channel", fixed: Some("{\"mode\": null}"), undo: Some("channel.add-member"), before: Some("/api/channels/{id}/members/{bot}"), ..A },
    Action { name: "channel.read", route: "/api/channels/{id}/read", label: "Mark read", icon: Some("check"), help: "Mark a channel read up to a message (through), else its newest", input: &[opt("through", Json)], access: Owner, ..A },
    Action { name: "channel.pause", route: "/api/channels/{id}/pause", label: "Pause", icon: Some("stop"), help: "Pause or resume a channel's deliveries: while paused, messages still post and no bot is woken", input: &[f("paused", Bool)], access: Owner, ..A },
    Action { name: "channel.mute", route: "/api/channels/{id}/mute", label: "Mute", icon: Some("bell-off"), help: "Mute or unmute a channel or DM for you: while muted, you still read and post there and its bots get every delivery as before, but new messages add no unread mark, count or Today row unless a bot asks or mentions you", input: &[f("muted", Bool)], access: Owner, ..A },
    Action { name: "channel.block", route: "/api/channels/{id}/block", label: "Block", help: "Block or unblock a DM: its bots can no longer post in it; you still can", input: &[f("blocked", Bool)], access: Owner, ..A },
    Action { name: "channel.allow-more", route: "/api/channels/{id}/allow-more", label: "Allow 10 more", icon: Some("play"), help: "Let 10 more deliveries pass the channel's loop guard (hop and hourly limits); the newest message's skipped deliveries go now", effect: Starts, access: Owner, ..A },
    Action { name: "channel.cancel-scheduled", method: "DELETE", route: "/api/channels/{id}/scheduled/{n}", label: "Cancel", icon: Some("x"), help: "Cancel a bot's scheduled post in a channel or DM (n from the channel page; bots schedule with `launchpad channel post|dm --in|--at` and cancel their own with `launchpad channel cancel-scheduled <n>`); it is never posted", confirm: Some("Deletes the scheduled post: it is never posted."), access: Owner, ..A },
    Action { name: "channel.digest", route: "/api/channels/{id}/digest", label: "Send digests now", icon: Some("send"), help: "Send this channel's members in mode digest their digest now (the messages since they last read), instead of waiting for the daily one at 08:00", effect: Starts, access: Owner, ..A },
    Action { name: "channel.rename", method: "PATCH", route: "/api/channels/{id}", label: "Rename", icon: Some("pencil"), help: "Rename a named channel or change its purpose", input: &[opt("name", Text), opt("purpose", Text)], undo: Some("channel.rename"), before: Some("/api/channels/{id}/about"), ..A },
    Action { name: "channel.archive", route: "/api/channels/{id}/archive", label: "Archive", icon: Some("archive"), help: "Archive or bring back a channel or DM: archived, nobody posts and its history stays", input: &[f("archived", Bool)], access: Owner, ..A },
    Action { name: "channel.delete", method: "DELETE", route: "/api/channels/{id}", label: "Delete", icon: Some("trash"), help: "Delete a channel or DM with its messages", confirm: Some("Deletes {name} and all its messages."), access: Owner, ..A },
    // Workspaces and tools.
    Action { name: "workspace.create", route: "/api/workspaces", label: "New workspace", icon: Some("plus"), help: "Create a workspace", input: &[f("name", Text)], undo: Some("workspace.delete"), ..A },
    Action { name: "workspace.rename", method: "PATCH", route: "/api/workspaces/{id}", label: "Rename", icon: Some("pencil"), help: "Rename a workspace", input: &[f("name", Text)], undo: Some("workspace.rename"), before: Some("/api/workspaces/{id}"), ..A },
    Action { name: "workspace.delete", method: "DELETE", route: "/api/workspaces/{id}", label: "Delete workspace", icon: Some("trash"), help: "Delete an empty workspace and its bots (not the last one)", confirm: Some("Deletes the empty workspace {name} and its bots."), ..A },
    // Settings → Devices → Phone notifications: the Firebase key Home wakes the Android app with.
    Action { name: "push.key", method: "PUT", route: "/api/phone/push/key", label: "Save on Home", help: "Save the Firebase service-account key (key: the whole JSON file) that Home uses to wake the Android app; a wake-up never carries any text", input: &[f("key", Text)], confirm: Some("Saves this key in Home's vault: Home then wakes your registered phones through Firebase."), effect: Machine, access: Owner, ..A },
    Action { name: "push.test", route: "/api/phone/push/test", label: "Send a test wake-up", icon: Some("send"), help: "Wake every registered phone once: each shows a test notification", effect: Checks, access: Owner, ..A },
    Action { name: "push.remove", method: "DELETE", route: "/api/phone/push/key", label: "Delete the key", icon: Some("trash"), help: "Delete the Firebase service-account key from Home: phones get no more wake-ups", confirm: Some("Deletes the Firebase key from Home. Phones no longer get notifications until you save a key again."), effect: Machine, access: Owner, ..A },
    Action { name: "tools.defaults", method: "PUT", route: "/api/tool-defaults/{scope}/{id}", label: "Save", help: "Set the default tools of a workspace or a folder ({scope}: workspace or folder): {\"<tool id or mcp:<server id>>\": true or false}; the folder wins over the workspace, a bot's bot.toml [tools] over both; only tools granted to the workspace apply", input: &[f("tools", Json)], undo: Some("tools.defaults"), before: Some("/api/tool-defaults/{scope}/{id}"), ..A },
    Action { name: "mcp.tool", method: "PUT", route: "/api/mcp/{id}/tools/{tool}", label: "Switch", help: "Switch one tool of an MCP server on or off", input: &[f("on", Bool)], ..A },
    // Folder settings and the shared settings of config.toml (the closed list below; the rest of config.toml stays the owner's).
    Action { name: "folder.clone", route: "/api/projects/clone", label: "Add to Launchpad", icon: Some("folder"), help: "Clone a GitHub repository (owner/name or URL) from GitHub into a new folder of a workspace (Home mirrors it; runners clone it on first use)", input: &[f("repo", Text), f("workspace_id", Text)], access: Owner, ..A },
    // A new, empty Git folder of Home (no remote, no GitHub, no path or setup command): admin may create one in its
    // own workspace; cloning stays the owner's.
    Action { name: "folder.create-git", route: "/api/projects/git", label: "New Git folder", icon: Some("folder"), help: "Create a new, empty Git folder in a workspace: a Git repository on Home without a remote (main starts with one empty commit); agents then work on it in worktrees. An agent's folder goes into its own workspace", input: &[f("name", Text), opt("workspace_id", Text)], ..A },
    Action { name: "github.review-choice", method: "PUT", route: "/api/ext/review-choice", label: "Remember", help: "Remember the bot and preset of Review with agent on a repository's GitHub lists (Chrome extension)", input: &[f("repo", Text), f("workspace", Text), opt("bot", Text), opt("preset", Text)], access: Owner, ..A },
    Action { name: "folder.set", method: "PATCH", route: "/api/projects/{id}", label: "Save", icon: Some("folder"), help: "Change a folder's name, description, When done (keep, merge, local_pr, draft_pr or open_pr), workspace or default bot (bot: a bot ID of its workspace for the Chrome extension's Ask on its GitHub pages; empty clears it)", input: &[opt("name", Text), opt("description", Text), opt("when_done", Text), opt("workspace_id", Text), opt("bot", Text)], undo: Some("folder.set"), before: Some("/api/projects/{id}"), ..A },
    Action { name: "settings.set", method: "PATCH", route: "/api/settings/shared", label: "Save", icon: Some("preset"), help: "Change the shared settings: presets (the whole list of {id, label, text, bots, agent}), worktrees_keep", input: &[opt("presets", Json), opt("worktrees_keep", Json)], card: Some("preset"), undo: Some("settings.set"), before: Some("/api/settings/shared"), ..A },
    // A profile's non-secret defaults. Profiles are Launchpad-wide (every workspace's sessions and bots use them);
    // env, login, billing, cost and spending limits, the model list URL, harness and ID stay the owner's.
    Action { name: "profile.set", method: "PATCH", route: "/api/settings/profiles/{id}", label: "Save", icon: Some("preset"), help: "Change a profile's default model and effort (Launchpad-wide: every workspace's new sessions, and bot threads that use the profile's defaults, which continue in a new thread at their next message); an empty value clears it (the harness's default); the effort must be one the model offers; env, login, billing, limits and the model list stay the owner's", input: &[opt("model", Text), opt("effort", Text)], undo: Some("profile.set"), before: Some("/api/settings/profiles/{id}"), ..A },
    // Agent branches.
    Action { name: "worktree.discard", route: "/api/worktrees/{id}/discard", label: "Discard", icon: Some("trash"), help: "Delete an agent's worktree folder and branch, including uncommitted work, and archive its agents when it is their main folder; the branch tip stays restorable on Home for 30 days", confirm: Some("Deletes {what}. The branch tip stays restorable on Home for 30 days."), access: Owner, ..A },
    Action { name: "worktree.fix-merge", route: "/api/worktrees/{id}/fix", label: "Fix merge", icon: Some("branch"), help: "Send a merge that stopped (conflict or failure) back to the agent: it rebases, resolves and asks to merge again", effect: Starts, ..A },
    // The owner's clicks: GitHub, other people, credentials and money.
    Action { name: "worktree.merge", route: "/api/worktrees/{id}/merge", label: "Merge", icon: Some("branch"), help: "Merge an agent's branch into its base branch", input: &[f("head", Text)], confirm: Some("Home pushes the reviewed commit of {branch} to {base} on the folder's remote, then fast-forwards {base} on Home. If {branch} changed since, nothing is merged; if {base} moved, the agent rebases first."), effect: Outside, access: Owner, ready: true, ..A },
    Action { name: "local-pr.publish", route: "/api/predrafts/{id}/publish", label: "Publish", icon: Some("pr"), help: "Publish a Local PR to GitHub", input: &[f("remote_branch", Text), f("draft", Bool), f("head", Text), opt("fork", Bool)], confirm: Some("Home pushes the reviewed commit and opens the pull request on GitHub."), effect: Outside, access: Owner, ready: true, ..A },
    Action { name: "local-pr.update-published", route: "/api/predrafts/{id}/update-published", label: "Update on GitHub", icon: Some("pr"), help: "Change the title and description of a published Local PR's pull request on GitHub, and the Local PR's with them (only pull requests Launchpad published, never a linked one); the description gets the attribution line like Publish", input: &[opt("title", Text), opt("body", Text)], confirm: Some("Home changes the pull request's title and description on GitHub."), effect: Outside, access: Owner, ..A },
    Action { name: "local-pr.publish-set", route: "/api/predrafts/{id}/publish-set", label: "Publish all", icon: Some("pr"), help: "Publish every open Local PR of a change set to GitHub, each at its reviewed commit (heads: {Local PR ID: commit})", input: &[f("draft", Bool), f("heads", Json)], confirm: Some("Home pushes the reviewed commit of each open Local PR of the set, opens the pull requests on GitHub and links them to each other. If a branch changed since review, nothing is published."), effect: Outside, access: Owner, ..A },
    Action { name: "local-pr.link-pr", route: "/api/predrafts/{id}/link-pr", label: "Link pull request", icon: Some("pr"), help: "Link a pull request opened outside Launchpad to an open Local PR (pr: its number or URL in the folder's repository; head: the branch commit you reviewed). Home reads it on GitHub and links it only when it targets the Local PR's base branch and its head is that commit, the branch's tip on Home; it pushes nothing and never pushes to it later. Once GitHub merged it (merge, squash or rebase), the Local PR reads as merged", input: &[f("pr", Text), f("head", Text)], confirm: Some("Home reads this pull request on GitHub and links it to the Local PR when its repository, base branch and head commit match. Nothing is pushed or posted."), effect: Record, access: Owner, ready: true, ..A },
    Action { name: "local-pr.merge", route: "/api/predrafts/{id}/merge-local", label: "Merge", icon: Some("branch"), help: "Merge a Local PR into its base branch", input: &[f("head", Text)], confirm: Some("Home pushes the reviewed commit to {base} on the folder's remote, then fast-forwards {base} on Home; if the remote refuses the push, nothing is merged."), effect: Outside, access: Owner, ready: true, ..A },
    // Discard closes a Local PR that is no longer wanted (it leaves Today): the owner's button, or an admin session
    // when the owner asks; Undo reopens it at the branch tip it had.
    Action { name: "local-pr.discard", route: "/api/predrafts/{id}/discard", label: "Discard", icon: Some("trash"), help: "Discard (close) an open Local PR so it leaves Today: archives it and deletes its branch on Home (the tip stays restorable for 30 days); refused while its agent works or a publication has started", confirm: Some("Deletes the branch on Home. Its tip stays restorable from the folder page for 30 days."), undo: Some("local-pr.reopen"), ready: true, ..A },
    Action { name: "local-pr.reopen", route: "/api/predrafts/{id}/reopen", label: "Reopen", icon: Some("pr"), help: "Reopen a discarded Local PR: restores its branch on Home at the tip it had when it was discarded (kept 30 days) and lists it again", ..A },
    Action { name: "suggestion.post", route: "/api/suggestions/{id}/post", label: "Post", icon: Some("comment"), help: "Post an agent's suggested comment or review to GitHub, then run its actions once (actions: the list to run, unticked ones left out; absent, the agent's; close: add or remove the close, Comment and close); an action-only suggestion runs its actions alone", input: &[opt("close", Bool), opt("actions", Json)], confirm: Some("Posts this to GitHub as you."), effect: Outside, access: Owner, ready: true, ..A },
    Action { name: "branch.push-to-pr", route: "/api/items/{owner}/{repo}/{number}/push-branch", label: "Push to this PR", icon: Some("branch"), help: "Push an agent's Launchpad branch (branch: lp/…) at the commit you saw (head) to an existing pull request's branch on GitHub: its repository, or the fork when its author lets maintainers edit; fast-forward only, never forced. Home then pushes the agent's next commits there too", input: &[f("branch", Text), f("head", Text)], confirm: Some("Home pushes {branch} at the commit you saw to this pull request's branch on GitHub (fast-forward only, never forced), then keeps pushing the agent's new commits there."), effect: Outside, access: Owner, ready: true, ..A },
    Action { name: "github.run-ci", route: "/api/items/{owner}/{repo}/{number}/run-ci", label: "Approve and run workflows", icon: Some("play"), help: "Approve the GitHub Actions runs that wait for a maintainer on a pull request's head (pull requests from forks)", input: &[f("head", Text)], confirm: Some("Runs this pull request's code in the repository's GitHub Actions."), effect: Outside, access: Owner, ready: true, ..A },
    Action { name: "suggestion.discard", method: "DELETE", route: "/api/suggestions/{id}", label: "Discard", help: "Discard a suggestion", confirm: Some("Deletes the local suggestion."), access: Owner, ..A },
    Action { name: "suggestion.edit", method: "PATCH", route: "/api/suggestions/{id}", label: "Save", icon: Some("pencil"), help: "Change a waiting suggestion's text before posting it (body; event: COMMENT, APPROVE or REQUEST_CHANGES; comments: the line comments to keep, each at its place with its text, rewritten or as written; merge, run_ci; close: whether Post also closes the item; actions: what Post also runs on the item; absent keeps them)", input: &[f("body", Text), f("event", Text), opt("comments", Json), opt("merge", Text), opt("run_ci", Bool), opt("close", Bool), opt("actions", Json)], access: Owner, ..A },
    Action { name: "note.write", route: "/api/notes/owner", label: "Add private note", icon: Some("comment"), help: "Add your private note to a GitHub item's timeline in Launchpad (never posted) and send it to the agents that wrote notes or suggestions about it there, else to its default bot (repo: owner/name; number; workspace; reply_to: a note ID)", input: &[f("repo", Text), f("number", Json), f("workspace", Text), f("body", Text), opt("reply_to", Text)], effect: Starts, access: Owner, ..A },
    Action { name: "note.read", route: "/api/notes/read", label: "Mark read", icon: Some("check"), help: "Mark the agents' private notes of a GitHub item read (repo: owner/name; number), or every note of the workspace when neither is given", input: &[f("workspace", Text), opt("repo", Text), opt("number", Json)], access: Owner, ..A },
    Action { name: "note.delete", method: "DELETE", route: "/api/notes/{id}", label: "Delete note", icon: Some("trash"), help: "Delete a private note of a GitHub item's timeline", confirm: Some("Deletes this note."), access: Owner, ..A },
    // The feedback ledger (Chrome extension): what happened to a suggestion on GitHub. Owner only: these record the owner's own GitHub forms.
    Action { name: "suggestion.shown", route: "/api/suggestions/{id}/shown", label: "Shown", icon: Some("eye"), help: "Record that a suggestion was shown on GitHub for 1.5 s (part: body or line:<path>:<line>:<side>)", input: &[opt("part", Text)], access: Owner, ..A },
    Action { name: "suggestion.inserted", route: "/api/suggestions/{id}/inserted", label: "Insert", icon: Some("comment"), help: "Record that a suggestion was inserted into GitHub's own comment box (text: what was inserted, without the attribution line); claims it like Post, so Launchpad's Post then says Inserted on GitHub", input: &[opt("part", Text), opt("text", Text)], access: Owner, ..A },
    Action { name: "suggestion.posted-by-hand", route: "/api/suggestions/{id}/posted-by-hand", label: "Posted by hand", icon: Some("check"), help: "Record that GitHub's own form was submitted with an inserted suggestion (text: the final text); Home then looks for the comment on GitHub to confirm it", input: &[opt("part", Text), f("text", Text)], access: Owner, ..A },
    Action { name: "suggestion.dismiss-comment", route: "/api/suggestions/{id}/dismiss-comment", label: "Dismiss", icon: Some("x"), help: "Dismiss one line comment of a suggested review (part: line:<path>:<line>:<side>); it is left out when the review is posted", input: &[f("part", Text)], access: Owner, ..A },
    Action { name: "outbox.send", route: "/api/outbox/{id}/send", label: "Send", icon: Some("send"), help: "Send an Outbox message", confirm: Some("Sends this message to {connection} now."), effect: Outside, access: Owner, ready: true, ..A },
    Action { name: "outbox.discard", route: "/api/outbox/{id}/discard", label: "Discard", help: "Discard an Outbox message", confirm: Some("Moves the draft to Discarded."), access: Owner, ..A },
    // Settings → Devices: paired browsers, the Chrome extension and the desktop companion. Clients delete their own
    // device with `DELETE /api/devices/self` (no button).
    Action { name: "device.revoke", method: "DELETE", route: "/api/devices/{id}", label: "Revoke", icon: Some("x"), help: "Revoke a paired device (browser, Chrome extension, desktop companion or Android app): it is signed out and its live streams close", confirm: Some("Signs {name} out of Launchpad and closes its live streams. It needs pairing again."), effect: Machine, access: Owner, ..A },
    Action { name: "slack.profile-save", method: "PUT", route: "/api/slack/profiles/{handle}", label: "Save on Home", help: "Create or change a Slack profile ({handle}: lowercase letters, digits and dashes): its name, description, token (xoxp- or xoxb-; required for a new one, kept when left out), on by default for every session, and whether messages end with Launchpad's attribution line", input: &[f("name", Text), f("description", Text), f("token", Text), f("default_on", Bool), f("footer", Bool)], access: Owner, ..A },
    Action { name: "slack.profile-delete", method: "DELETE", route: "/api/slack/profiles/{handle}", label: "Delete", icon: Some("trash"), help: "Delete a Slack profile and its token from Home; agents lose it at once", confirm: Some("Deletes the profile and its token from Home. Agents lose it at once. The token stays valid in Slack until you remove the app."), access: Owner, ..A },
    Action { name: "slack.profile-test", route: "/api/slack/profiles/{handle}/test", label: "Test", icon: Some("check"), help: "Ask Slack who a profile's token acts as (auth.test); the token never leaves Home", effect: Checks, access: Owner, ..A },
    Action { name: "vault.refresh-login", route: "/api/vault/entries/{id}/refresh", label: "Refresh", icon: Some("refresh"), help: "Refresh a harness login of Home's vault now (a Codex login gets new tokens whatever their expiry; a Claude token is checked with a one-word Haiku request unless a valid reading exists) and read its quota again; says when the login needs a new sign-in", effect: Checks, access: Owner, ..A },
    Action { name: "tool.save", method: "PUT", route: "/api/vault/tools/{id}", label: "Add tool", help: "Add a tool to Home's registry", confirm: Some("Adds this tool and its credential hosts to Home's registry."), effect: Machine, access: Owner, card: Some("tool"), ..A },
];

pub fn get(name: &str) -> Option<&'static Action> {
    ACTIONS.iter().find(|a| a.name == name)
}

/// The action a card kind runs when it is accepted.
pub fn for_card(kind: &str) -> Option<&'static Action> {
    // Older BOT.md cards apply like the bot-files cards that replaced them.
    let kind = if kind == "bot_instructions" { "bot_files" } else { kind };
    ACTIONS.iter().find(|a| a.card == Some(kind))
}

/// The action a request runs (path without its query). The match is exact, like the router's: a trailing slash,
/// an extra or an empty segment matches no action.
pub fn find(method: &str, path: &str) -> Option<&'static Action> {
    let p: Vec<&str> = path.split('/').collect();
    ACTIONS.iter().find(|a| {
        let r: Vec<&str> = a.route.split('/').collect();
        a.method.eq_ignore_ascii_case(method)
            && r.len() == p.len()
            && r.iter().zip(&p).all(|(r, p)| if r.starts_with('{') { !p.is_empty() } else { r == p })
    })
}

/// The route's `{param}` names, in order.
pub fn params(a: &Action) -> Vec<&'static str> {
    a.route.split('/').filter_map(|s| s.strip_prefix('{')?.strip_suffix('}')).collect()
}

/// The route with its parameters filled in order (each one a single path segment).
pub fn path(a: &Action, args: &[&str]) -> anyhow::Result<String> {
    let names = params(a);
    anyhow::ensure!(args.len() == names.len(), "{} takes {}", a.name, usage_args(a));
    let mut it = args.iter();
    let mut out = String::new();
    for seg in a.route.split('/').skip(1) {
        out.push('/');
        if seg.starts_with('{') {
            let v = it.next().expect("counted");
            anyhow::ensure!(!v.is_empty() && !v.contains('/') && !matches!(*v, "." | ".."), "{v:?} is not an id");
            out.push_str(&encode(v));
        } else {
            out.push_str(seg);
        }
    }
    Ok(out)
}

fn encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

fn usage_args(a: &Action) -> String {
    let p = params(a);
    if p.is_empty() { "no argument".into() } else { p.iter().map(|n| format!("<{n}>")).collect::<Vec<_>>().join(" ") }
}

/// The action's CLI usage: `<cli> <group> <verb> <id> --text <text> [--effort <text>]`.
pub fn usage(cli: &str, a: &Action) -> String {
    let (group, verb) = a.name.split_once('.').unwrap_or((a.name, ""));
    let mut out = format!("{cli} {group} {verb}");
    for p in params(a) {
        out.push_str(&format!(" <{p}>"));
    }
    for fl in a.input {
        let v = match fl.kind { Text => "text", Bool => "true|false", Json => "json" };
        out.push_str(&if fl.optional { format!(" [--{} <{v}>]", fl.name) } else { format!(" --{} <{v}>", fl.name) });
    }
    if a.body_file.is_some() {
        out.push_str(" --body-file <file.json>");
    }
    out
}

/// The TypeScript list for the UI's `ActionButton` (`web/src/gen/actions.ts`).
pub fn typescript() -> String {
    let names: Vec<String> = ACTIONS.iter().map(|a| format!("{:?}", a.name)).collect();
    let mut out = String::from("// Generated by `launchpad gen-types` from crates/core/src/actions.rs. Do not edit.\nimport type { Action } from \"./Action\";\n\n");
    out.push_str(&format!("export type ActionName =\n  | {};\n\n", names.join("\n  | ")));
    out.push_str("export const ACTIONS: Record<ActionName, Action> = {\n");
    for a in ACTIONS {
        out.push_str(&format!("  {:?}: {},\n", a.name, serde_json::to_string(a).expect("serializable")));
    }
    out.push_str("};\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The GitHub publications whose attribution line says "posted manually" (`attribution::Origin::Manual`) run
    /// only on the owner's click: never an admin agent's direct action, never a card an agent or auto mode accepts
    /// (only a ready card, which runs from its own button). Agents publish through `launchpad pr publish`, which
    /// the route stamps as automatic.
    #[test]
    fn manual_publications_are_owner_clicks_only() {
        for name in ["suggestion.post", "local-pr.publish", "local-pr.publish-set", "local-pr.update-published"] {
            let a = get(name).unwrap();
            assert_eq!(a.access, ActionAccess::Owner, "{name}");
            assert_eq!(a.card, None, "{name}");
        }
    }
}

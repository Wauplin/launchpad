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
    /// Reaches GitHub or other people (merge, publish, post, send, run CI, Inbox done): the owner's click only.
    Outside,
    /// Money, credentials or the machine: the owner's click only.
    Machine,
    /// Installs a newer Launchpad on Home's Space (Home restarts; agents continue on their runners). Admin may run
    /// it when the owner asks; it has a confirmation and no undo.
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
    // Sessions.
    Action { name: "session.launch", route: "/api/sessions", label: "Launch", icon: Some("play"), help: "Launch an agent", body_file: Some("a launch spec"), effect: Starts, card: Some("launch"), ..A },
    Action { name: "session.message", route: "/api/sessions/{id}/input", label: "Send", icon: Some("send"), help: "Send a message to an agent session", input: &[f("text", Text)], effect: Starts, card: Some("message"), ..A },
    Action { name: "session.answer", route: "/api/sessions/{id}/answer", label: "Answer", help: "Answer an agent's open question (request_id from `sessions show`); a permission request (Allow or Deny) is the owner's", input: &[f("request_id", Text), f("answers", Json)], effect: Starts, ..A },
    Action { name: "session.stop", route: "/api/sessions/{id}/interrupt", label: "Stop", icon: Some("stop"), help: "Stop an agent's turn", ..A },
    Action { name: "session.archive", route: "/api/sessions/{id}/close", label: "Archive", icon: Some("archive"), help: "Archive an agent session (stops it first)", ..A },
    Action { name: "session.unarchive", route: "/api/sessions/{id}/unarchive", label: "Unarchive", icon: Some("archive"), help: "Bring back an archived agent (Done, or Stopped if it was not Done); its next message resumes it", ..A },
    Action { name: "session.resume", route: "/api/sessions/{id}/resume", label: "Resume", icon: Some("play"), help: "Resume an interrupted or failed agent", input: &[opt("continue_turn", Bool)], effect: Starts, ..A },
    Action { name: "session.resume-subscription", route: "/api/sessions/{id}/resume-subscription", label: "Resume on subscription", icon: Some("play"), help: "Resume a quota-paused agent on its subscription", effect: Starts, ..A },
    Action { name: "session.rename", route: "/api/sessions/{id}/rename", label: "Rename", icon: Some("pencil"), help: "Rename an agent session", input: &[f("title", Text)], undo: Some("session.rename"), before: Some("/api/sessions/{id}"), ..A },
    Action { name: "session.model", route: "/api/sessions/{id}/model", label: "Change model", help: "Change an agent's model or effort for its next turns", input: &[opt("model", Text), opt("effort", Text)], ..A },
    Action { name: "session.permissions", route: "/api/sessions/{id}/permissions", label: "Change permissions", help: "Change an agent's permission mode (yolo, approvals or auto); an agent may only make it stricter", input: &[f("mode", Text)], ..A },
    Action { name: "session.follow-up", route: "/api/sessions/{id}/follow-up", label: "Follow up", icon: Some("clock"), help: "Schedule a follow-up message to an agent", input: &[f("message", Text), f("conditions", Json)], effect: Starts, ..A },
    Action { name: "session.seen", route: "/api/sessions/{id}/seen", label: "Mark seen", icon: Some("eye"), help: "Mark an agent as seen", ..A },
    Action { name: "session.unread", route: "/api/sessions/{id}/unread", label: "Mark unread", icon: Some("eye"), help: "Mark an agent as unread", ..A },
    Action { name: "session.resume-paid", route: "/api/sessions/{id}/resume-paid", label: "Resume with paid usage", help: "Resume a quota-paused agent with paid usage", confirm: Some("Continues on paid usage, billed to the profile's account."), effect: Machine, access: Owner, ..A },
    // Bots.
    Action { name: "bot.create", route: "/api/bots", label: "Create bot", icon: Some("plus"), help: "Create a bot (it starts nothing; an agent's bot gets at most propose access)", body_file: Some("{\"workspace_id\", \"name\", \"instructions\", \"profile\", \"folder\"}"), card: Some("bot"), undo: Some("bot.delete"), ..A },
    Action { name: "bot.update", method: "PATCH", route: "/api/bots/{id}", label: "Save", help: "Change a bot's settings, its BOT.md or its routine files (an agent never raises `launchpad`, turns `inbox_done` on or loosens `permission_mode`)", body_file: Some("{\"set\": {<bot.toml fields to change, for example \"proposals\": \"manual\"; null puts back the default>}}, {\"settings\": <all of bot.toml>}, {\"instructions\": \"<whole BOT.md>\"} or {\"files\": {\"routines/<name>.toml\": \"<whole file>\" or null to delete it}}"), card: Some("bot_files"), undo: Some("bot.update"), before: Some("/api/bots/{id}"), ..A },
    Action { name: "bot.delete", method: "DELETE", route: "/api/bots/{id}", label: "Delete bot", icon: Some("trash"), help: "Delete a bot, its memory and its routines", confirm: Some("Deletes the bot, its memory and its routines. Its sessions stay in Agents."), ..A },
    Action { name: "bot.message", route: "/api/bots/{id}/message", label: "Send", icon: Some("send"), help: "Send a message to a bot's thread (new_session starts a new session of it, reason says why in the chat); context (optional) adds read-only folders and agent traces: {\"context_projects\": [{\"project_id\": \"…\", \"branch\": null}], \"agent_traces\": [{\"session_id\": \"…\", \"through_seq\": 12}]}", input: &[f("text", Text), opt("context", Json), opt("new_session", Bool), opt("reason", Text)], effect: Starts, ..A },
    Action { name: "bot.pause", route: "/api/bots/{id}/pause", label: "Pause", help: "Pause or resume a bot (and its routines and hooks)", input: &[f("paused", Bool)], undo: Some("bot.pause"), before: Some("/api/bots/{id}"), ..A },
    Action { name: "bot.cancel-queued", method: "DELETE", route: "/api/bots/{id}/queued/{n}", label: "Cancel", icon: Some("x"), help: "Cancel a hook or Telegram message that waits for a busy bot (n from GET /api/bots/{id}/queued); it is never sent", confirm: Some("Deletes the waiting message: it is never sent."), ..A },
    Action { name: "bot.restore", route: "/api/bots/{id}/restore/{change}", label: "Restore", icon: Some("refresh"), help: "Restore an earlier version of a bot's files", ..A },
    Action { name: "bot.add-routine", route: "/api/bots/{id}/routines", label: "Add routine", icon: Some("plus"), help: "Add a routine to a bot", body_file: Some("{\"title\", \"instructions\", \"trigger\", \"schedule\", \"conditions\"}"), card: Some("routine"), undo: Some("routine.delete"), ..A },
    // Routines and scheduled launches.
    Action { name: "routine.create", route: "/api/launch/crons", label: "Create routine", icon: Some("clock"), help: "Create a routine that launches a spec", body_file: Some("{\"spec\": <launch spec>, \"schedule\", \"conditions\", \"trigger\", \"title\"}"), card: Some("cron"), undo: Some("routine.delete"), ..A },
    Action { name: "routine.update", method: "PATCH", route: "/api/launch/crons/{id}", label: "Save", help: "Change a routine (schedule, conditions, paused, title, instructions)", body_file: Some("the fields to change"), undo: Some("routine.update"), before: Some("/api/launch/crons/{id}"), ..A },
    Action { name: "routine.delete", method: "DELETE", route: "/api/launch/crons/{id}", label: "Delete", icon: Some("trash"), help: "Delete a routine", confirm: Some("Deletes the routine. Its past runs stay in Agents."), ..A },
    Action { name: "routine.run", route: "/api/launch/crons/{id}/run", label: "Run now", icon: Some("play"), help: "Run a routine now", effect: Starts, ..A },
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
    Action { name: "runner.check-network", route: "/api/runners/{id}/network-check", label: "Check network", icon: Some("refresh"), help: "Check a runner's sandbox network: capture, DNS, UDP policy, IPv4 and IPv6, runner CA", effect: Checks, access: Owner, ..A },
    // Agent CLI updates on the runners (`launchpad_setup::cli_update`). Installing on a machine is the owner's
    // click; agents may check and mark results read.
    Action { name: "cli-update.check", route: "/api/cli-updates/check", label: "Check now", icon: Some("refresh"), help: "Read the newest agent CLI versions on npm now and update the runners (same major version) or ask for approval (new major version)", effect: Checks, ..A },
    Action { name: "cli-update.approve", route: "/api/cli-updates/{id}/approve", label: "Approve", icon: Some("download"), help: "Install a new major version of an agent CLI on the runners that wait for it (each one once no agent of it is in a turn)", confirm: Some("Installs {version} on the runners that wait for it, once no agent there is in a turn."), effect: Machine, access: Owner, ..A },
    Action { name: "cli-update.retry", route: "/api/cli-updates/{id}/retry", label: "Retry", icon: Some("refresh"), help: "Install an agent CLI update again on the runners where it did not install", confirm: Some("Installs {version} again on the runners where it did not install, once no agent there is in a turn."), effect: Machine, access: Owner, ..A },
    Action { name: "cli-update.dismiss", route: "/api/cli-updates/{id}/dismiss", label: "Mark read", icon: Some("check"), help: "Remove an agent CLI update from Today (a major version: skip it)", ..A },
    Action { name: "cli-update.auto", method: "PUT", route: "/api/cli-updates/auto", label: "Update automatically", help: "Turn automatic updates of one agent CLI on or off on every runner", input: &[f("harness", Text), f("on", Bool)], confirm: Some("Changes whether the runners update this CLI on their own."), effect: Machine, access: Owner, ..A },
    // Home updates (a Home on a Hugging Face Space: `launchpad_setup::home_update`). Admin runs them when the owner
    // asks. Updating the runners afterwards stays the owner's: the button's `update_runners` field is not an input
    // here, so an agent's request or a ready card cannot set it (`agent_auth` refuses fields outside the input).
    Action { name: "home.check-update", route: "/api/home/update/check", label: "Check for updates", icon: Some("refresh"), help: "Check whether a newer Launchpad exists for Home's Space (release or main, as deployed): deployed, latest and available", effect: Checks, ..A },
    Action { name: "home.update", route: "/api/home/update", label: "Update Home", icon: Some("download"), help: "Install a newer Launchpad on Home's Space: the newest of the deployed channel, or of `channel` (release or main); `version`, when given, must be that newest one (from home check-update). The call returns once the Space starts its build; Home restarts a few minutes later, while agents continue on their runners: wait, then check again", input: &[opt("version", Text), opt("channel", Text)], confirm: Some("Home's Space installs {version}, rebuilds and restarts: Home is unavailable for a few minutes. Running agents continue on their runners."), effect: Installs, ready: true, ..A },
    // Bot channels and DMs (`launchpad_channels::channel`). A post from an agent's token posts as its bot, only in a
    // channel it is a member of. Members and new channels are admin (recorded with Undo); the rest is the owner's.
    Action { name: "channel.post", route: "/api/channels/{id}/messages", label: "Send", icon: Some("send"), help: "Post the owner's own message in a bot channel or DM (bots post with `launchpad channel post` and `launchpad dm`); it wakes the bots it mentions and the members in mode all", input: &[f("text", Text), opt("reply_to", Json), opt("ask_owner", Bool)], effect: Starts, access: Owner, ..A },
    Action { name: "channel.dm", route: "/api/channels/dm", label: "Open DM", icon: Some("comment"), help: "Open the DM of two bots of one workspace (created on first use; both bots are members in mode all; a bot opens its own with `launchpad dm`)", input: &[f("bots", Json)], access: Owner, ..A },
    Action { name: "channel.create", route: "/api/channels", label: "New channel", icon: Some("plus"), help: "Create a named bot channel in your workspace (name: lowercase letters, digits and dashes; members: bot slugs or ids, comma-separated or a JSON list); its member bots are woken when mentioned (mode mentions) unless a member is {\"bot\": \"release\", \"mode\": \"all\"}; workspace_id: an agent's is its own, the owner names one", input: &[f("name", Text), opt("purpose", Text), opt("members", Json), opt("workspace_id", Text)], card: Some("channel"), ..A },
    Action { name: "channel.add-member", method: "PUT", route: "/api/channels/{id}/members/{bot}", label: "Add member", icon: Some("plus"), help: "Add a bot of the channel's workspace to a named channel, or change its mode: all (every message), mentions, digest or muted (null removes it)", input: &[f("mode", Text)], card: Some("channel_join"), undo: Some("channel.add-member"), before: Some("/api/channels/{id}/members/{bot}"), ..A },
    Action { name: "channel.set-mode", method: "PUT", route: "/api/channels/{id}/members/{bot}/mode", label: "Change mode", help: "Change when a member bot of a named channel is woken: all (every message), mentions, digest (a daily summary) or muted", input: &[f("mode", Text)], undo: Some("channel.set-mode"), before: Some("/api/channels/{id}/members/{bot}"), ..A },
    Action { name: "channel.remove-member", route: "/api/channels/{id}/members/{bot}/remove", label: "Remove", icon: Some("x"), help: "Remove a bot from a named channel", fixed: Some("{\"mode\": null}"), undo: Some("channel.add-member"), before: Some("/api/channels/{id}/members/{bot}"), ..A },
    Action { name: "channel.read", route: "/api/channels/{id}/read", label: "Mark read", icon: Some("check"), help: "Mark a channel read up to a message (through), else its newest", input: &[opt("through", Json)], access: Owner, ..A },
    Action { name: "channel.pause", route: "/api/channels/{id}/pause", label: "Pause", icon: Some("stop"), help: "Pause or resume a channel's deliveries: while paused, messages still post and no bot is woken", input: &[f("paused", Bool)], access: Owner, ..A },
    Action { name: "channel.block", route: "/api/channels/{id}/block", label: "Block", help: "Block or unblock a DM: its bots can no longer post in it; you still can", input: &[f("blocked", Bool)], access: Owner, ..A },
    Action { name: "channel.allow-more", route: "/api/channels/{id}/allow-more", label: "Allow 10 more", icon: Some("play"), help: "Let 10 more deliveries pass the channel's loop guard (hop and hourly limits); the newest message's skipped deliveries go now", effect: Starts, access: Owner, ..A },
    Action { name: "channel.digest", route: "/api/channels/{id}/digest", label: "Send digests now", icon: Some("send"), help: "Send this channel's members in mode digest their digest now (the messages since they last read), instead of waiting for the daily one at 08:00", effect: Starts, access: Owner, ..A },
    Action { name: "channel.rename", method: "PATCH", route: "/api/channels/{id}", label: "Rename", icon: Some("pencil"), help: "Rename a named channel or change its purpose", input: &[opt("name", Text), opt("purpose", Text)], access: Owner, ..A },
    Action { name: "channel.archive", route: "/api/channels/{id}/archive", label: "Archive", icon: Some("archive"), help: "Archive or bring back a channel or DM: archived, nobody posts and its history stays", input: &[f("archived", Bool)], access: Owner, ..A },
    Action { name: "channel.delete", method: "DELETE", route: "/api/channels/{id}", label: "Delete", icon: Some("trash"), help: "Delete a channel or DM with its messages", confirm: Some("Deletes {name} and all its messages."), access: Owner, ..A },
    // Workspaces and tools.
    Action { name: "workspace.create", route: "/api/workspaces", label: "New workspace", icon: Some("plus"), help: "Create a workspace", input: &[f("name", Text)], undo: Some("workspace.delete"), ..A },
    Action { name: "workspace.rename", method: "PATCH", route: "/api/workspaces/{id}", label: "Rename", icon: Some("pencil"), help: "Rename a workspace", input: &[f("name", Text)], undo: Some("workspace.rename"), before: Some("/api/workspaces/{id}"), ..A },
    Action { name: "workspace.delete", method: "DELETE", route: "/api/workspaces/{id}", label: "Delete workspace", icon: Some("trash"), help: "Delete an empty workspace, its bots and its Telegram bot (not the last one)", confirm: Some("Deletes the empty workspace {name}, its bots and its Telegram setup."), ..A },
    // Each workspace's own Telegram bot and chat (Settings → Workspace). Connections are the owner's: never admin.
    Action { name: "telegram.token", method: "PUT", route: "/api/connections/telegram/{workspace}/token", label: "Save on Home", help: "Save a workspace's Telegram bot token from @BotFather (checked with Telegram; a bot already used by another workspace is refused) and show a binding code", input: &[f("token", Text)], access: Owner, ..A },
    Action { name: "telegram.code", route: "/api/connections/telegram/{workspace}/code", label: "Show a binding code", icon: Some("plus"), help: "A new single-use code: the next private chat that sends /start <code> to the workspace's Telegram bot is bound instead of the current one", access: Owner, ..A },
    Action { name: "telegram.set", method: "PATCH", route: "/api/connections/telegram/{workspace}", label: "Save", help: "Switch a workspace's Telegram notifications (notify) and session titles in them (titles)", input: &[opt("notify", Bool), opt("titles", Bool)], access: Owner, ..A },
    Action { name: "telegram.test", route: "/api/connections/telegram/{workspace}/test", label: "Send test message", icon: Some("send"), help: "Send a test message to a workspace's bound Telegram chat", effect: Checks, access: Owner, ..A },
    Action { name: "telegram.remove", method: "DELETE", route: "/api/connections/telegram/{workspace}", label: "Delete Telegram setup", icon: Some("trash"), help: "Delete a workspace's Telegram bot token and bound chat", confirm: Some("Deletes the bot token and the bound chat of this workspace from Home. Telegram is no longer read or answered for it."), effect: Machine, access: Owner, ..A },
    Action { name: "tools.defaults", method: "PUT", route: "/api/tool-defaults/{scope}/{id}", label: "Save", help: "Set the default tools of a workspace or a folder ({scope}: workspace or folder): {\"<tool id or mcp:<server id>>\": true or false}; the folder wins over the workspace, a bot's bot.toml [tools] over both; only tools granted to the workspace apply", input: &[f("tools", Json)], undo: Some("tools.defaults"), before: Some("/api/tool-defaults/{scope}/{id}"), ..A },
    Action { name: "mcp.tool", method: "PUT", route: "/api/mcp/{id}/tools/{tool}", label: "Switch", help: "Switch one tool of an MCP server on or off", input: &[f("on", Bool)], ..A },
    // Folder settings and the shared settings of config.toml (the closed list below; the rest of config.toml stays the owner's).
    Action { name: "folder.set", method: "PATCH", route: "/api/projects/{id}", label: "Save", icon: Some("folder"), help: "Change a folder's name, description, When done (keep, merge, local_pr, draft_pr or open_pr), workspace or default bot (bot: a bot ID of its workspace for the Chrome extension's Ask on its GitHub pages; empty clears it)", input: &[opt("name", Text), opt("description", Text), opt("when_done", Text), opt("workspace_id", Text), opt("bot", Text)], undo: Some("folder.set"), before: Some("/api/projects/{id}"), ..A },
    Action { name: "settings.set", method: "PATCH", route: "/api/settings/shared", label: "Save", icon: Some("preset"), help: "Change the shared settings: presets (the whole list of {id, label, text, bots, agent}), github_poll_interval_seconds, worktrees_keep", input: &[opt("presets", Json), opt("github_poll_interval_seconds", Json), opt("worktrees_keep", Json)], card: Some("preset"), undo: Some("settings.set"), before: Some("/api/settings/shared"), ..A },
    // Agent branches.
    Action { name: "worktree.discard", route: "/api/worktrees/{id}/discard", label: "Discard", icon: Some("trash"), help: "Delete an agent's worktree folder and branch, including uncommitted work, and archive its agents when it is their main folder; the branch tip stays restorable on Home for 30 days", confirm: Some("Deletes {what}. The branch tip stays restorable on Home for 30 days."), access: Owner, ..A },
    Action { name: "worktree.fix-merge", route: "/api/worktrees/{id}/fix", label: "Fix merge", icon: Some("branch"), help: "Send a merge that stopped (conflict or failure) back to the agent: it rebases, resolves and asks to merge again", effect: Starts, ..A },
    // The owner's clicks: GitHub, other people, credentials and money.
    Action { name: "worktree.merge", route: "/api/worktrees/{id}/merge", label: "Merge", icon: Some("branch"), help: "Merge an agent's branch into its base branch", input: &[f("head", Text)], confirm: Some("Home pushes the reviewed commit of {branch} to {base} on the folder's remote, then fast-forwards {base} on Home. If {branch} changed since, nothing is merged; if {base} moved, the agent rebases first."), effect: Outside, access: Owner, ready: true, ..A },
    Action { name: "local-pr.publish", route: "/api/predrafts/{id}/publish", label: "Publish", icon: Some("pr"), help: "Publish a Local PR to GitHub", input: &[f("remote_branch", Text), f("draft", Bool), f("head", Text), opt("fork", Bool)], confirm: Some("Home pushes the reviewed commit and opens the pull request on GitHub."), effect: Outside, access: Owner, ready: true, ..A },
    Action { name: "local-pr.publish-set", route: "/api/predrafts/{id}/publish-set", label: "Publish all", icon: Some("pr"), help: "Publish every open Local PR of a change set to GitHub, each at its reviewed commit (heads: {Local PR ID: commit})", input: &[f("draft", Bool), f("heads", Json)], confirm: Some("Home pushes the reviewed commit of each open Local PR of the set, opens the pull requests on GitHub and links them to each other. If a branch changed since review, nothing is published."), effect: Outside, access: Owner, ..A },
    Action { name: "local-pr.merge", route: "/api/predrafts/{id}/merge-local", label: "Merge", icon: Some("branch"), help: "Merge a Local PR into its base branch", input: &[f("head", Text)], confirm: Some("Home pushes the reviewed commit to {base} on the folder's remote, then fast-forwards {base} on Home; if the remote refuses the push, nothing is merged."), effect: Outside, access: Owner, ready: true, ..A },
    Action { name: "suggestion.post", route: "/api/suggestions/{id}/post", label: "Post", icon: Some("comment"), help: "Post an agent's suggested comment or review to GitHub", confirm: Some("Posts this text to GitHub as you."), effect: Outside, access: Owner, ready: true, ..A },
    Action { name: "github.run-ci", route: "/api/items/{owner}/{repo}/{number}/run-ci", label: "Approve and run workflows", icon: Some("play"), help: "Approve the GitHub Actions runs that wait for a maintainer on a pull request's head (pull requests from forks)", input: &[f("head", Text)], confirm: Some("Runs this pull request's code in the repository's GitHub Actions."), effect: Outside, access: Owner, ready: true, ..A },
    Action { name: "suggestion.discard", method: "DELETE", route: "/api/suggestions/{id}", label: "Discard", help: "Discard a suggestion", confirm: Some("Deletes the local suggestion."), access: Owner, ..A },
    // The feedback ledger (Chrome extension): what happened to a suggestion on GitHub. Owner only: these record the owner's own GitHub forms.
    Action { name: "suggestion.shown", route: "/api/suggestions/{id}/shown", label: "Shown", icon: Some("eye"), help: "Record that a suggestion was shown on GitHub for 1.5 s (part: body or line:<path>:<line>:<side>)", input: &[opt("part", Text)], access: Owner, ..A },
    Action { name: "suggestion.inserted", route: "/api/suggestions/{id}/inserted", label: "Insert", icon: Some("comment"), help: "Record that a suggestion was inserted into GitHub's own comment box (text: what was inserted, without the attribution line); claims it like Post, so Launchpad's Post then says Inserted on GitHub", input: &[opt("part", Text), opt("text", Text)], access: Owner, ..A },
    Action { name: "suggestion.posted-by-hand", route: "/api/suggestions/{id}/posted-by-hand", label: "Posted by hand", icon: Some("check"), help: "Record that GitHub's own form was submitted with an inserted suggestion (text: the final text); Home then looks for the comment on GitHub to confirm it", input: &[opt("part", Text), f("text", Text)], access: Owner, ..A },
    Action { name: "suggestion.dismiss-comment", route: "/api/suggestions/{id}/dismiss-comment", label: "Dismiss", icon: Some("x"), help: "Dismiss one line comment of a suggested review (part: line:<path>:<line>:<side>); it is left out when the review is posted", input: &[f("part", Text)], access: Owner, ..A },
    Action { name: "outbox.send", route: "/api/outbox/{id}/send", label: "Send", icon: Some("send"), help: "Send an Outbox message", confirm: Some("Sends this message to {connection} now."), effect: Outside, access: Owner, ready: true, ..A },
    Action { name: "outbox.discard", route: "/api/outbox/{id}/discard", label: "Discard", help: "Discard an Outbox message", confirm: Some("Moves the draft to Discarded."), access: Owner, ..A },
    Action { name: "inbox.done", route: "/api/inbox/{id}/action", label: "Done", icon: Some("check"), help: "Mark an Inbox thread done, here and on GitHub", fixed: Some("{\"action\": \"done\"}"), effect: Outside, access: Owner, undo: Some("inbox.restore"), ready: true, ..A },
    Action { name: "inbox.restore", route: "/api/inbox/restore", label: "Restore", icon: Some("refresh"), help: "Bring Inbox threads marked done back", input: &[f("ids", Json)], access: Owner, ..A },
    Action { name: "vault.refresh-login", route: "/api/vault/entries/{id}/refresh", label: "Refresh", icon: Some("refresh"), help: "Refresh a harness login of Home's vault now (a Codex login gets new tokens whatever their expiry; a Claude token is checked with a one-token request) and read its quota again; says when the login needs a new sign-in", effect: Checks, access: Owner, ..A },
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

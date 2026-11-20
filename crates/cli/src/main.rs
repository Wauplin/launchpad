//! `launchpad`: the daemon and the agent-facing commands.
//!
//! The environment comes from `LAUNCHPAD_ENV` (`prod` by default, `dev` for development).

mod extensions;
#[cfg(target_os = "linux")]
mod netcap;
mod folio;
mod oracle;
mod slack;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use launchpad_core::paths::Paths;

/// The daemon API, over its local port.
#[derive(Clone)]
struct Client {
    base: String,
    token: String,
    http: reqwest::Client,
}

impl Client {
    fn new(paths: &Paths) -> Result<Client> {
        // LAUNCHPAD_TOKEN: a restricted credential (a session's CLI token).
        let token = match std::env::var("LAUNCHPAD_TOKEN") { Ok(t) if !t.trim().is_empty() => t.trim().to_string(), _ => paths.read_token()? };
        Ok(Client { base: paths.base_url(), token, http: reqwest::Client::new() })
    }

    /// `path` includes the query. Returns the status and body text.
    async fn send(&self, method: &str, path: &str, body: Option<serde_json::Value>) -> Result<(u16, String)> {
        let mut req = self.http.request(reqwest::Method::from_bytes(method.as_bytes())?, format!("{}{path}", self.base)).bearer_auth(&self.token);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let res = req.send().await.context("is the daemon running?")?;
        let status = res.status().as_u16();
        Ok((status, res.text().await?))
    }

    async fn json(&self, method: &str, path: &str, body: Option<serde_json::Value>) -> Result<serde_json::Value> {
        let (status, text) = self.send(method, path, body).await?;
        let body: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text));
        if status == 503 && body["retryable"] == true {
            return Err(Retryable(failure(status, &body)).into());
        }
        anyhow::ensure!((200..300).contains(&status), "{}", failure(status, &body));
        Ok(body)
    }
}

/// A write Home refused for now (its maintenance during a planned update): nothing changed, try again later. The
/// command exits with `RETRY_EXIT`.
#[derive(Debug)]
struct Retryable(String);

impl std::fmt::Display for Retryable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Retryable {}

/// The exit code of a refusal to retry later (`EX_TEMPFAIL`).
const RETRY_EXIT: i32 = 75;

/// The reason of a failed request: its JSON `error`, else a short plain-text body (axum's input rejections), else
/// the status.
fn failure(status: u16, body: &serde_json::Value) -> String {
    match (body["error"].as_str(), body.as_str()) {
        (Some(e), _) => e.to_string(),
        (None, Some(t)) if !t.trim().is_empty() && t.len() <= 500 => t.trim().to_string(),
        _ => format!("request failed (HTTP {status})"),
    }
}

fn query(path: &str, pairs: &[(&str, &str)]) -> String {
    let mut url = reqwest::Url::parse(&format!("http://x{path}")).expect("valid path");
    url.query_pairs_mut().extend_pairs(pairs);
    format!("{}?{}", url.path(), url.query().unwrap_or(""))
}

#[derive(Parser)]
#[command(name = "launchpad", about = "Launchpad daemon and command-line tools")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Manage local pre-draft pull requests.
    Pr {
        #[command(subcommand)]
        command: Pr,
    },
    /// Link this session to a GitHub item and its workspace topic.
    Link {
        #[arg(long)]
        repo: String,
        #[arg(long)]
        number: u64,
        #[arg(long)]
        comment_url: Option<String>,
        #[arg(long, env = "LAUNCHPAD_SESSION_ID", hide_env_values = true)]
        session_id: String,
    },
    /// Add a private note to a GitHub issue's or pull request's timeline (Markdown, never posted): each run appends a
    /// note, shown in GitHub's timeline by the Chrome extension. `--suggest-file` attaches a suggested comment (posted
    /// only by the owner's click); `--reply-to-note` answers another note (the owner's).
    Note {
        #[arg(long)]
        repo: String,
        #[arg(long)]
        number: u64,
        #[arg(long)]
        body_file: std::path::PathBuf,
        /// A comment the owner could post on GitHub (saved like `suggest comment`, shown under the note with Post); never
        /// put such a draft in the note body.
        #[arg(long)]
        suggest_file: Option<std::path::PathBuf>,
        /// The suggested comment answers one comment (as `suggest comment --reply-to`).
        #[arg(long, requires = "suggest_file")]
        reply_to: Option<String>,
        /// What the owner's Post also does (as `suggest comment`): after the `--suggest-file` comment, or alone (an
        /// action-only suggestion under the note: a close without a comment…). Use them whenever the note recommends
        /// one.
        #[command(flatten)]
        actions: ActionFlags,
        /// The note this one answers (its ID).
        #[arg(long)]
        reply_to_note: Option<String>,
        #[arg(long, env = "LAUNCHPAD_SESSION_ID", hide_env_values = true)]
        session_id: String,
    },
    /// List the private notes of a GitHub issue or pull request in this session's workspace, oldest first, each with
    /// its `url`: put that link in a note's Markdown to point at the note.
    Notes {
        #[arg(long)]
        repo: String,
        #[arg(long)]
        number: u64,
    },
    /// End this turn and have Launchpad wake this session later, in the same conversation: instead of a long `sleep` in
    /// the shell or polling. Wake after a delay or at a time, or once other agents of this workspace reach a state
    /// (`--agents`, Done by default), or once a quota window resets; every condition given must hold (the conditions
    /// of a scheduled launch). Run it, then end your turn; the wake-up message brings back your note. A new sleep
    /// replaces the last.
    ///
    /// Examples: `sleep --in 30m`; `sleep --until 2026-11-14T15:00:00Z`; `sleep --agents <id1>,<id2> --timeout 2h`;
    /// `sleep --agents <id> --state done,failed,asking`; `sleep --quota claude:five_hour`.
    Sleep {
        /// Wake in this long: 45m, 2h, 1d (minutes without a unit).
        #[arg(long = "in", required_unless_present_any = ["until", "agents", "quota"])]
        delay: Option<String>,
        /// Wake at this time (RFC 3339 with its offset, such as 2026-11-14T15:00:00Z).
        #[arg(long)]
        until: Option<String>,
        /// Wake once each of these agents (session IDs of this workspace, comma-separated) is in one of `--state`.
        #[arg(long, value_delimiter = ',')]
        agents: Vec<String>,
        /// The states that count for `--agents` (any of them): done (default; Done, or archived after Done), asking,
        /// failed (or interrupted), stopped, quota_paused, sleeping, archived, working.
        #[arg(long, value_delimiter = ',', requires = "agents")]
        state: Vec<String>,
        /// Wake once a quota window resets: <harness>:five_hour or <harness>:weekly, such as claude:five_hour.
        #[arg(long)]
        quota: Option<String>,
        /// With `--agents` or `--quota`: wake at the latest after this long, even when the conditions do not hold: 45m, 2h, 1d.
        #[arg(long)]
        timeout: Option<String>,
        /// What to do when you wake up (comes back in the wake-up message).
        #[arg(long, default_value = "")]
        note: String,
        #[arg(long, env = "LAUNCHPAD_SESSION_ID", hide_env_values = true)]
        session_id: String,
    },
    /// Save a suggestion locally and link its session to the GitHub item and topic; never posts.
    Suggest {
        #[command(subcommand)]
        command: Suggest,
    },
    /// GitHub Actions of a pull request.
    Ci {
        #[command(subcommand)]
        command: Ci,
    },
    /// Send a message through one of the owner's connections: `telegram` goes to the owner's own chat of the session's workspace;
    /// other connections (Slack, Discord, webhooks) wait in the owner's Outbox until they click Send.
    Send {
        /// The connection name (Settings → Connections), or `telegram` (Settings → Workspace).
        connection: String,
        #[arg(long)]
        body_file: std::path::PathBuf,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        session_id: Option<String>,
    },
    /// Task worktrees managed by Launchpad (for agents).
    Work {
        #[command(subcommand)]
        command: Work,
    },
    /// Asset folders: large or generated files of a Git folder, stored by Launchpad instead of Git (for agents).
    Assets {
        #[command(subcommand)]
        command: Assets,
    },
    /// The workspace's knowledge base, the only memory agents have: read its files in $LAUNCHPAD_KB, write here.
    Kb {
        #[command(subcommand)]
        command: Kb,
    },
    /// Registered folders (JSON), optionally of one workspace.
    Folders {
        #[command(subcommand)]
        command: ListOnly,
    },
    /// A bot session's own memory (`bot memory`: read its notes in $LAUNCHPAD_BOT/memory, write here), and the
    /// short verbs of bot actions (`bot pause <id> --paused true`, admin access).
    Bot {
        #[command(subcommand)]
        command: BotCommand,
    },
    /// Bots of a workspace with their job, profile, folder and current session (JSON).
    Bots {
        #[command(subcommand)]
        command: ListOnly,
    },
    /// Agent sessions (JSON): `list` or `show <id>`.
    Sessions {
        #[command(subcommand)]
        command: Sessions,
    },
    /// Profiles without their environment, with the quota of each subscription login (JSON).
    Profiles {
        #[command(subcommand)]
        command: ListOnly,
    },
    /// Exit routes with their exit runners' states: online or offline, since when (JSON).
    Egress {
        #[command(subcommand)]
        command: ListOnly,
    },
    /// Prompt presets (JSON).
    Presets {
        #[command(subcommand)]
        command: ListOnly,
    },
    /// Scheduled launches and cron agents (JSON).
    Schedules {
        #[command(subcommand)]
        command: ListOnly,
    },
    /// Ask Launchpad proposals with ids, titles, states and contents (JSON).
    Proposals {
        #[command(subcommand)]
        command: ListOnly,
    },
    /// What waits for the owner today (JSON), optionally of one workspace.
    Today {
        #[arg(long)]
        workspace: Option<String>,
    },
    /// What sessions with admin access changed directly, with the values before (JSON).
    Activity {
        #[command(subcommand)]
        command: ListOnly,
    },
    /// Recent log lines of Home and runners, oldest first (sessions with admin access). Default: every source,
    /// the last hour, info and above.
    Logs {
        /// Only this runner (id or name).
        #[arg(long, conflicts_with_all = ["home", "local"])]
        runner: Option<String>,
        /// Only Home.
        #[arg(long, conflicts_with = "local")]
        home: bool,
        /// Only the daemon this command talks to (the runner of this session).
        #[arg(long)]
        local: bool,
        /// Lines about this session (its id, short id and Burst runners).
        #[arg(long)]
        session: Option<String>,
        /// 30m, 2h, 1d or Unix milliseconds.
        #[arg(long, default_value = "1h")]
        since: String,
        /// Lowest level: error, warn, info or debug.
        #[arg(long)]
        level: Option<String>,
        /// Only lines that contain this text.
        #[arg(long)]
        grep: Option<String>,
        #[arg(long, default_value_t = 300)]
        limit: usize,
        /// Print JSON instead of text lines.
        #[arg(long)]
        json: bool,
    },
    /// Everything failing now, by area, with the ids to investigate (JSON; admin access).
    Health,
    /// Failures Home noticed: sessions, Burst jobs, routine runs, scheduled launches, runners, restarts (JSON; admin access).
    Failures {
        /// 30m, 2h, 1d or Unix milliseconds (default: the last 24 hours).
        #[arg(long)]
        since: Option<String>,
    },
    /// Runners with connection, build, guard and open sessions (JSON; admin access).
    Runners {
        #[command(subcommand)]
        command: ListOnly,
    },
    /// Burst: `runs` (settings, why it cannot start, runs with their errors) or `job <runner id>` (the HF Job's stage and log) (JSON; admin access).
    Burst {
        #[command(subcommand)]
        command: Burst,
    },
    /// Home: uptime, recent starts (clean or not), backups and update state (JSON; admin access).
    Home,
    /// Tools, credential names and MCP servers with their last error, without secrets (JSON; admin access).
    Tools {
        #[command(subcommand)]
        command: ListOnly,
    },
    /// Connections and each workspace's Telegram link, without URLs or tokens (JSON; admin access).
    Connections {
        #[command(subcommand)]
        command: ListOnly,
    },
    /// Bot channels of this session's workspace (for bots): `list`, `read <name|@bot>`, `post <name> --body-file m.md
    /// [--attach <file>]… [--forward <message id>]…`, `files <name|@bot> <message id>` (downloads a message's files),
    /// `join <name>` (a card the owner accepts); and the short verbs of channel actions (`channel create <name>
    /// --purpose … --members a,b`, `channel edit <id|name> [--name …] [--purpose …]`,
    /// `channel add-member|set-mode|remove-member`, admin access).
    Channel {
        #[command(subcommand)]
        command: ChannelCommand,
    },
    /// The conversation of this bot thread (for bots): several bots may share one conversation with the owner, each in
    /// its own thread. `read [--since <id>] [--limit 50]` lists its messages (the owner's with the bots they went to,
    /// each bot's final answers, the messages bots sent each other), oldest first (JSON); you never receive them
    /// automatically.
    Conversation {
        #[command(subcommand)]
        command: ConversationCommand,
    },
    /// Write to another bot of this workspace in your DM with it (created on first use; for bots). Send files with
    /// `--attach <file>` (repeat it) and forward the files of an earlier message with `--forward <message id>`.
    Dm {
        /// The other bot: its slug or name, with or without `@`.
        bot: String,
        #[command(flatten)]
        post: ChannelPostArgs,
    },
    /// Oracles of this session's workspace: small Python checks Home runs to answer True, False or Failed (`list`,
    /// `show`, `run`, `history`; `test --file` runs a draft here in your jail; `save` at admin access). `run` and
    /// `test` exit 0 True, 1 False, 2 Failed, 3 for a CLI error.
    Oracle {
        /// Workspace ID (default: this session's workspace).
        #[arg(long, global = true)]
        workspace: Option<String>,
        #[command(subcommand)]
        command: oracle::OracleCommand,
    },
    /// Folios of this session's workspace: reports, plans and sketchbooks written as Markdown with typed blocks,
    /// rendered by Launchpad (`list`, `outline`, `read`, `create`, `write`, `append`, `patch`, `add-media`, `rm`,
    /// `check [--dir]`, `blocks`, `rules`, `feedback`, `items`, `reply`, `resolve`, `revs`). Exit 1 when a check has
    /// errors, 3 for a CLI error.
    Folio {
        /// Workspace ID (default: this session's workspace).
        #[arg(long, global = true)]
        workspace: Option<String>,
        #[command(subcommand)]
        command: folio::FolioCommand,
    },
    /// The hop count to send with `X-Launchpad-Hops` when this session calls an incoming hook of Launchpad (for agents).
    Hops,
    /// Slack with one of this session's Slack profiles: `launchpad slack --profile <handle> <command>`, the same commands
    /// and options as slackline (`launchpad slack --profile <handle> --help`). Home runs it with the profile's token
    /// and exits like slackline; `files upload` reads the file here, `files download -o` writes it here.
    #[command(disable_help_flag = true)]
    Slack {
        /// The profile's handle, from this session's instructions.
        #[arg(long)]
        profile: Option<String>,
        /// slackline's arguments.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 0..)]
        args: Vec<String>,
    },
    /// Store an inert proposal for the owner to start with a click; never starts anything.
    Propose {
        #[arg(value_parser = ["launch", "message", "schedule", "cron", "status", "preset", "tool", "bot", "routine", "bot-files", "bot-instructions", "channel", "oracle", "ready", "report"])]
        kind: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        note: Option<String>,
        /// Supersede a waiting, unstarted proposal in the same workspace.
        #[arg(long)]
        replaces: Option<String>,
        /// JSON file with the proposal fields (see the Launchpad CLI tool instructions).
        #[arg(long)]
        spec_file: std::path::PathBuf,
        /// The session writing it (defaults to LAUNCHPAD_SESSION_ID).
        #[arg(long)]
        session_id: Option<String>,
    },
    /// Call Launchpad's API (sessions with admin access to the Launchpad CLI tool): `launchpad api PATCH /api/bots/<id> --body-file b.json`.
    Api {
        /// GET, POST, PUT, PATCH or DELETE.
        method: String,
        /// The API path, starting with /api/.
        path: String,
        /// JSON file with the request body.
        #[arg(long)]
        body_file: Option<std::path::PathBuf>,
    },
    /// Run an action of Launchpad's action list (sessions with admin access): `launchpad do session.archive <id>`.
    /// Short verbs do the same: `launchpad session archive <id>`, `launchpad bot pause <id> --paused true`.
    Do {
        /// The action name, `<group>.<verb>`.
        action: String,
        /// Route ids in order, then `--<field> <value>` options and `--body-file <file.json>`.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    #[command(external_subcommand)]
    Verb(Vec<String>),
    /// Print the stored trace of an agent session, for use as reference context.
    Topics {
        #[arg(default_value = "list")]
        verb: String,
        id: Option<String>,
        #[arg(long)]
        workspace: Option<String>,
    },
    Trace {
        session_id: String,
        /// Last event included (defaults to the latest stored event).
        #[arg(long)]
        through: Option<i64>,
    },
    /// Prints the path of this runner's proxy CA certificate (created if needed), for a Local VM's system store.
    #[command(hide = true)]
    ProxyCa,
    /// Run the daemon (HTTP + WebSocket; loopback by default).
    Daemon {
        /// home (the default): records and services, no agents; runner (Linux only): agents and terminals, in its own
        /// folders (`~/.local/share/launchpad-runner`), paired with `launchpad connect`.
        #[arg(long, default_value = "home")]
        role: String,
    },
    /// Pair this Linux machine as a runner of a Home: shows a code to approve in Home's Settings → Home and runners.
    Connect {
        /// Home's address, for example https://<you>-launchpad.hf.space
        url: String,
        /// Name shown on Home (default: the host name).
        #[arg(long)]
        name: Option<String>,
    },
    /// Print the daemon status as JSON.
    Status,
    /// Print the address that signs a browser into the UI.
    Url,
    /// Print a five-minute pairing link and QR code for a browser or phone.
    Pair,
    /// Write the TypeScript types shared with the UI.
    #[command(hide = true)]
    GenTypes {
        #[arg(default_value = "web/src/gen")]
        out: std::path::PathBuf,
    },
}

#[derive(Subcommand)]
enum ListOnly {
    List {
        /// Workspace ID or name (default: every workspace).
        #[arg(long)]
        workspace: Option<String>,
    },
}

#[derive(Subcommand)]
enum Burst {
    Runs {
        /// Only the runs of this session.
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
    },
    Job { runner_id: String },
}

/// `2026-11-09T10:04:05Z` from Unix milliseconds.
fn utc(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[allow(clippy::too_many_arguments)]
async fn logs_command(paths: &Paths, runner: Option<String>, home: bool, local: bool, session: Option<String>, since: String, level: Option<String>, grep: Option<String>, limit: usize, json: bool) -> Result<()> {
    let source = if local { "local".to_string() } else if home { "home".into() } else { runner.unwrap_or_else(|| "all".into()) };
    let limit = limit.to_string();
    let mut pairs = vec![("source", source.as_str()), ("since", since.as_str()), ("limit", limit.as_str())];
    if let Some(s) = &session { pairs.push(("session", s)); }
    if let Some(l) = &level { pairs.push(("level", l)); }
    if let Some(g) = &grep { pairs.push(("grep", g)); }
    let body = Client::new(paths)?.json("GET", &query("/api/logs", &pairs), None).await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&body)?);
        return Ok(());
    }
    for l in body["lines"].as_array().into_iter().flatten() {
        println!("{} {:5} {}: {}", utc(l["at"].as_i64().unwrap_or(0)), l["level"].as_str().unwrap_or(""), l["source"].as_str().unwrap_or(""), l["message"].as_str().unwrap_or(""));
    }
    for e in body["errors"].as_array().into_iter().flatten() {
        eprintln!("{}: {}", e["source"].as_str().unwrap_or("?"), e["error"].as_str().unwrap_or("?"));
    }
    if body["lines"].as_array().is_none_or(|l| l.is_empty()) {
        eprintln!("No line matches (try a longer --since, or --level debug).");
    }
    Ok(())
}

#[derive(Subcommand)]
enum Sessions {
    List {
        #[arg(long)]
        workspace: Option<String>,
    },
    Show { session_id: String },
}

/// Read-only context commands: GET `/api/context/<what>`.
async fn list_command(paths: &Paths, base: &str, workspace: Option<String>) -> Result<()> {
    let path = match workspace { Some(w) => query(base, &[("workspace", &w)]), None => base.into() };
    let body = Client::new(paths)?.json("GET", &path, None).await?;
    println!("{}", serde_json::to_string_pretty(&body)?);
    Ok(())
}

async fn context_command(paths: &Paths, what: &str, workspace: Option<String>) -> Result<()> {
    let path = match workspace { Some(w) => query(&format!("/api/context/{what}"), &[("workspace", &w)]), None => format!("/api/context/{what}") };
    let body = Client::new(paths)?.json("GET", &path, None).await?;
    println!("{}", serde_json::to_string_pretty(&body)?);
    Ok(())
}

#[derive(Subcommand)]
enum Suggest {
    /// A comment the owner posts with one click (with its actions, `--close`…).
    Comment(SuggestArgs),
    /// A review of a pull request at `--head` (with `--merge`, `--run-ci` and its actions).
    Review(SuggestArgs),
    /// Only actions, no text (a close without a comment, `--request-reviewers`…): the owner's Post runs them.
    Action(SuggestActionArgs),
}
#[derive(Subcommand)]
enum Ci {
    /// Approve the GitHub Actions runs of a pull request that wait for a maintainer (pull requests from forks), at
    /// the head you inspected, without the owner's click. Approve only after reading the whole diff, never when it
    /// adds or changes workflows, actions or other CI configuration (Home refuses those itself), or when the pull
    /// request looks like spam or malicious. Home records each approval as a private note on the item.
    Approve {
        #[arg(long)]
        repo: String,
        #[arg(long)]
        number: u64,
        /// The 40-character head commit you read: refused when the pull request moved past it.
        #[arg(long)]
        head: String,
        #[arg(long, env = "LAUNCHPAD_SESSION_ID", hide_env_values = true)]
        session_id: String,
    },
}
/// How a recommended close ends an issue (pull requests close without a reason).
#[derive(Clone, Copy, Debug, PartialEq, clap::ValueEnum)]
enum CloseArg {
    Completed,
    NotPlanned,
}
/// What the owner's Post also does on a GitHub issue or pull request, after the comment or review (or alone, `suggest
/// action`). Every action you recommend carries its flag: the owner applies it with the same click.
#[derive(clap::Args, Debug, Default, PartialEq)]
struct ActionFlags {
    /// Close the issue or pull request after posting (a pull request without merging). An issue may take a reason:
    /// `--close completed` or `--close not-planned`.
    #[arg(long, num_args = 0..=1, value_name = "REASON")]
    close: Option<Option<CloseArg>>,
    /// Close the issue as a duplicate of issue N (the comment then starts with `Duplicate of #N`, which GitHub links).
    #[arg(long, value_name = "N", conflicts_with = "close")]
    duplicate_of: Option<u64>,
    /// Reopen the closed issue or pull request.
    #[arg(long, conflicts_with_all = ["close", "duplicate_of"])]
    reopen: bool,
    /// Pull requests: request reviews from these logins and teams (`org/team`), comma-separated.
    #[arg(long, value_delimiter = ',', value_name = "LOGINS")]
    request_reviewers: Vec<String>,
    /// Pull requests: mark the draft ready for review.
    #[arg(long, conflicts_with = "convert_to_draft")]
    ready_for_review: bool,
    /// Pull requests: convert to a draft.
    #[arg(long)]
    convert_to_draft: bool,
    /// Pull requests: update the branch with its base branch (GitHub's Update branch).
    #[arg(long)]
    update_branch: bool,
}
impl ActionFlags {
    /// The actions as Home stores them (`ItemAction` JSON).
    fn to_json(&self) -> Vec<serde_json::Value> {
        use serde_json::json;
        let mut out = vec![];
        if self.reopen {
            out.push(json!({"kind": "reopen"}));
        }
        if self.update_branch {
            out.push(json!({"kind": "update_branch"}));
        }
        if self.ready_for_review {
            out.push(json!({"kind": "ready_for_review"}));
        }
        if self.convert_to_draft {
            out.push(json!({"kind": "convert_to_draft"}));
        }
        if !self.request_reviewers.is_empty() {
            let (reviewers, teams) = launchpad_github::suggestions::split_reviewers(&self.request_reviewers);
            out.push(json!({"kind": "request_reviewers", "reviewers": reviewers, "team_reviewers": teams}));
        }
        if let Some(n) = self.duplicate_of {
            out.push(json!({"kind": "close", "reason": "duplicate", "duplicate_of": n}));
        } else if let Some(reason) = self.close {
            out.push(json!({"kind": "close", "reason": reason.map(|r| match r {
                CloseArg::Completed => "completed",
                CloseArg::NotPlanned => "not_planned",
            })}));
        }
        out
    }
}
#[derive(clap::Args)]
struct SuggestActionArgs {
    #[arg(long)]
    repo: String,
    #[arg(long)]
    number: u64,
    #[command(flatten)]
    actions: ActionFlags,
    #[arg(long)]
    session_id: Option<String>,
}
#[derive(clap::Args)]
struct SuggestArgs {
    #[arg(
        long,
        required_unless_present = "predraft",
        conflicts_with = "predraft",
        requires = "number"
    )]
    repo: Option<String>,
    #[arg(long, requires = "repo")]
    number: Option<u64>,
    #[arg(long, conflicts_with_all = ["repo", "number"])]
    predraft: Option<String>,
    #[arg(long)]
    body_file: std::path::PathBuf,
    /// Reviews only: a JSON array of line comments posted with the review, each
    /// `{"path": "src/a.rs", "line": 12, "side": "RIGHT", "start_line": 10, "body": "…"}` (`side` LEFT for a removed
    /// line, RIGHT by default; `start_line` optional, for a range such as a multi-line ```suggestion block).
    #[arg(long)]
    comments_file: Option<std::path::PathBuf>,
    #[arg(long, default_value = "comment", value_parser = ["comment", "approve", "request-changes"])]
    event: String,
    #[arg(long)]
    head: Option<String>,
    /// With `--event approve`: the owner's Post click also merges the pull request at the reviewed head.
    #[arg(long, value_parser = ["squash", "merge", "rebase"])]
    merge: Option<String>,
    /// The owner's Post click also approves the reviewed head's GitHub Actions runs that wait for a maintainer.
    #[arg(long, conflicts_with = "merge")]
    run_ci: bool,
    /// Comments only: the comment this comment answers, `issuecomment-<id>` or `discussion_r<id>` (the anchor of the
    /// comment's URL), the URL itself, or a review thread ID (`PRRT_…`). A review thread's comment gets its reply in
    /// that thread; a conversation comment gets a new comment that links it.
    #[arg(long, requires = "repo")]
    reply_to: Option<String>,
    /// What the owner's Post also does on the GitHub item (not on Local PRs): `--close` makes it Comment and close.
    #[command(flatten)]
    actions: ActionFlags,
    #[arg(long)]
    session_id: Option<String>,
}
async fn suggest_command(paths: &Paths, command: Suggest) -> Result<()> {
    let (kind, q) = match command {
        Suggest::Comment(q) => ("comment", q),
        Suggest::Review(q) => ("review", q),
        Suggest::Action(q) => {
            let actions = q.actions.to_json();
            anyhow::ensure!(!actions.is_empty(), "name at least one action: --close [completed|not-planned], --duplicate-of <n>, --reopen, --request-reviewers <logins>, --ready-for-review, --convert-to-draft or --update-branch");
            let target = serde_json::json!({"kind":"git_hub","repo":q.repo,"number":q.number});
            let body = Client::new(paths)?.json("POST", "/api/suggestions", Some(serde_json::json!({"target":target,"kind":"action","body":"","actions":actions,"session_id":q.session_id}))).await?;
            println!("{}", serde_json::to_string_pretty(&body)?);
            return Ok(());
        }
    };
    let actions = q.actions.to_json();
    let comments: serde_json::Value = match &q.comments_file {
        Some(file) => serde_json::from_str(&std::fs::read_to_string(file)?).with_context(|| format!("{} is not a JSON array of line comments", file.display()))?,
        None => serde_json::json!([]),
    };
    anyhow::ensure!(comments.is_array(), "--comments-file must hold a JSON array of line comments");
    anyhow::ensure!(kind == "review" || comments.as_array().is_some_and(|c| c.is_empty()), "only reviews have line comments");
    anyhow::ensure!(kind == "comment" || q.reply_to.is_none(), "--reply-to is for comments: `suggest comment --reply-to …`");
    anyhow::ensure!(kind == "comment" || (q.actions.close.is_none() && q.actions.duplicate_of.is_none()), "--close is for comments: `suggest comment --close` (a review cannot close a pull request)");
    anyhow::ensure!(q.predraft.is_none() || actions.is_empty(), "actions are for GitHub issues and pull requests, not Local PRs");
    let target = match q.predraft {
        Some(id) => serde_json::json!({"kind":"predraft","id":id}),
        None => serde_json::json!({"kind":"git_hub","repo":q.repo,"number":q.number}),
    };
    let body = Client::new(paths)?
        .json("POST", "/api/suggestions", Some(serde_json::json!({"target":target,"kind":kind,"body":std::fs::read_to_string(q.body_file)?,"comments":comments,"event":q.event.replace('-', "_").to_uppercase(),"head":q.head,"merge":q.merge,"run_ci":q.run_ci,"reply_to":q.reply_to,"actions":actions,"session_id":q.session_id})))
        .await?;
    println!("{}", serde_json::to_string_pretty(&body)?);
    Ok(())
}

#[derive(Subcommand)]
enum Work {
    /// Ask Launchpad to merge this session's branches into their base branches after the turn ends.
    Finish {
        #[arg(long, env = "LAUNCHPAD_SESSION_ID", hide_env_values = true)]
        session_id: String,
    },
    /// Create a dedicated worktree of another registered project for this session, and print its folder.
    AddProject {
        #[arg(long, env = "LAUNCHPAD_SESSION_ID", hide_env_values = true)]
        session_id: String,
        /// Project name or ID.
        #[arg(long)]
        project: String,
    },
}
#[derive(Subcommand)]
enum Kb {
    /// Print the folder of the knowledge base (read-only files).
    Path,
    /// Lines of the knowledge base that contain every word.
    Search { words: Vec<String> },
    /// Create or replace a note with stdin (or --file). Paths are relative to the knowledge base.
    Write {
        path: String,
        #[arg(long)]
        file: Option<std::path::PathBuf>,
        #[arg(long, env = "LAUNCHPAD_SESSION_ID", hide_env_values = true)]
        session_id: String,
    },
    /// Add stdin (or --file, or the text arguments) to the end of a note.
    Append {
        path: String,
        text: Vec<String>,
        #[arg(long)]
        file: Option<std::path::PathBuf>,
        #[arg(long, env = "LAUNCHPAD_SESSION_ID", hide_env_values = true)]
        session_id: String,
    },
    /// Copy an image, video or audio file into media/ (or --as <path>).
    AddMedia {
        file: std::path::PathBuf,
        #[arg(long = "as")]
        to: Option<String>,
        #[arg(long, env = "LAUNCHPAD_SESSION_ID", hide_env_values = true)]
        session_id: String,
    },
    /// Remove a note (restorable for 30 days).
    Rm {
        path: String,
        #[arg(long, env = "LAUNCHPAD_SESSION_ID", hide_env_values = true)]
        session_id: String,
    },
}

#[derive(Subcommand)]
enum ChannelCommand {
    /// The channels of this workspace and your bot's DMs, with its membership, mode and unread messages (JSON).
    List,
    /// Messages of a channel (`release-train`) or of your DM with a bot (`@release`), oldest first (JSON): after
    /// `--since`, else the newest. Moves your bot's last-read mark.
    Read {
        channel: String,
        #[arg(long)]
        since: Option<i64>,
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    /// Post your bot's message (Markdown, at most 8 KB; `@<bot slug>` wakes a bot, `@owner` asks the owner), with
    /// files: `--attach <file>` (repeat it) and `--forward <message id>` (the files of a message you read).
    Post {
        /// The channel's name, or `@<bot>` for your DM with it.
        channel: String,
        #[command(flatten)]
        post: ChannelPostArgs,
    },
    /// Download the files of a message (`files` in `channel read`) into a folder (default: a new folder under /tmp) and
    /// print their paths (JSON). Forward them with `--attach <path>`, or all of them with `--forward <message id>`.
    Files {
        /// The channel's name, or `@<bot>` for your DM with it.
        channel: String,
        /// The message ID.
        message: i64,
        /// Where to write the files (created when missing).
        #[arg(long)]
        dir: Option<std::path::PathBuf>,
    },
    /// Ask to join a named channel of this workspace: files a card "<your bot> asks to join #<name>" that the owner
    /// accepts (only member bots read and post).
    Join {
        channel: String,
        /// When your bot is woken: all (every message), mentions (the default), digest or muted.
        #[arg(long)]
        mode: Option<String>,
        /// Why your bot wants to join, one or two sentences.
        #[arg(long)]
        note: Option<String>,
        #[arg(long, env = "LAUNCHPAD_SESSION_ID", hide_env_values = true)]
        session_id: String,
    },
    /// Rename a named channel or change its purpose (admin access; recorded in the activity list with Undo).
    Edit {
        /// The channel's ID or name (from `channel list`).
        channel: String,
        /// The new name: lowercase letters, digits and dashes, like release-train.
        #[arg(long)]
        name: Option<String>,
        /// The new purpose, one line.
        #[arg(long)]
        purpose: Option<String>,
    },
    #[command(external_subcommand)]
    Verb(Vec<String>),
}

#[derive(clap::Args)]
struct ChannelPostArgs {
    /// The message file (`-` reads stdin). Optional when the post carries files.
    #[arg(long)]
    body_file: Option<std::path::PathBuf>,
    /// A file to send with the message (at most 5 MB; up to 10 files, 20 MB together). Repeat it for several. A
    /// file you received is listed with its local path: attach that path to forward it.
    #[arg(long = "attach", value_name = "FILE")]
    attach: Vec<std::path::PathBuf>,
    /// Forward the files of an earlier message (its ID, from `channel read` or a delivery) of a channel or DM your
    /// bot reads. Repeat it for several.
    #[arg(long = "forward", value_name = "MESSAGE_ID")]
    forward: Vec<i64>,
    /// The message this one answers.
    #[arg(long)]
    reply_to: Option<i64>,
    /// Ask the owner: the message waits for them in Today.
    #[arg(long)]
    ask_owner: bool,
    #[arg(long, env = "LAUNCHPAD_SESSION_ID", hide_env_values = true)]
    session_id: String,
}

/// The largest file `--attach` sends (Home refuses larger ones too).
const CHANNEL_FILE_MAX: u64 = 5 * 1024 * 1024;

/// Pure: the MIME type Home stores a file under, from its extension (images Launchpad shows, else a few text and
/// document types, else `application/octet-stream`).
fn file_mime(path: &std::path::Path) -> &'static str {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "csv" => "text/csv",
        "md" | "markdown" => "text/markdown",
        "txt" | "log" | "rs" | "py" | "ts" | "tsx" | "js" | "toml" | "yaml" | "yml" | "sh" | "diff" | "patch" | "html" | "css" | "xml" | "svg" => "text/plain",
        _ => "application/octet-stream",
    }
}

/// `launchpad channel post` and `launchpad dm`: Home posts as this session's bot. Each `--attach` file goes to Home
/// first (one request per file, `/api/channels/agent/upload`); the post names the stored references.
async fn channel_post(paths: &Paths, channel: String, a: ChannelPostArgs) -> Result<()> {
    use base64::Engine;
    anyhow::ensure!(a.body_file.is_some() || !a.attach.is_empty() || !a.forward.is_empty(), "give the message with --body-file <file> (or attach files with --attach <file>, --forward <message id>)");
    let text = match &a.body_file {
        Some(f) => String::from_utf8(read_input((f.as_os_str() != "-").then_some(f.as_path()))?).context("the message is not UTF-8 text")?,
        None => String::new(),
    };
    let client = Client::new(paths)?;
    let mut files = vec![];
    for path in &a.attach {
        let meta = std::fs::metadata(path).with_context(|| format!("read {}", path.display()))?;
        anyhow::ensure!(meta.is_file(), "{} is not a file", path.display());
        anyhow::ensure!(meta.len() <= CHANNEL_FILE_MAX, "{} is larger than 5 MB: send a smaller file", path.display());
        let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
        let body = serde_json::json!({"session_id": a.session_id, "name": name, "mime": file_mime(path), "data": base64::engine::general_purpose::STANDARD.encode(&bytes)});
        let up = client.json("POST", "/api/channels/agent/upload", Some(body)).await.with_context(|| format!("upload {}", path.display()))?;
        files.push(serde_json::json!({"name": up["name"], "reference": up["reference"]}));
    }
    let body = serde_json::json!({"session_id": a.session_id, "channel": channel, "text": text, "reply_to": a.reply_to, "ask_owner": a.ask_owner, "files": files, "forward": a.forward});
    let v = client.json("POST", "/api/channels/agent/post", Some(body)).await?;
    println!("{}", serde_json::to_string_pretty(&v)?);
    Ok(())
}

/// Pure: the file name `channel files` writes: the last path part with only safe characters, numbered when a
/// message carries two files of one name.
fn download_name(name: &str, index: usize, taken: &std::collections::HashSet<String>) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let clean: String = base.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }).take(120).collect();
    let clean = if clean.trim_matches('.').is_empty() { format!("file-{index}") } else { clean };
    if taken.contains(&clean) { format!("{index}-{clean}") } else { clean }
}

/// `launchpad channel files`: each file of a message, one request per file, written into `dir`.
async fn channel_files(paths: &Paths, channel: String, message: i64, dir: Option<std::path::PathBuf>) -> Result<()> {
    use base64::Engine;
    let client = Client::new(paths)?;
    let dir = dir.unwrap_or_else(|| std::env::temp_dir().join("launchpad-channel-files").join(message.to_string()));
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let (mut out, mut taken) = (vec![], std::collections::HashSet::new());
    let mut index = 0usize;
    loop {
        let (m, i) = (message.to_string(), index.to_string());
        let v = client.json("GET", &query("/api/channels/agent/file", &[("channel", channel.as_str()), ("message", m.as_str()), ("index", i.as_str())]), None).await?;
        let bytes = base64::engine::general_purpose::STANDARD.decode(v["data"].as_str().context("Home sent no file")?)?;
        let name = download_name(v["name"].as_str().unwrap_or("file"), index, &taken);
        let path = dir.join(&name);
        std::fs::write(&path, &bytes).with_context(|| format!("write {}", path.display()))?;
        taken.insert(name.clone());
        out.push(serde_json::json!({"name": v["name"], "path": path, "size": bytes.len()}));
        index += 1;
        if index >= v["count"].as_u64().unwrap_or(0) as usize {
            break;
        }
    }
    println!("{}", serde_json::to_string_pretty(&serde_json::json!({"message": message, "files": out}))?);
    Ok(())
}

/// Pure: `channel create <name> [--purpose …] [--members a,b]` as the `channel.create` action's options (a
/// positional name becomes `--name`, a comma list of members a JSON list).
fn channel_create_args(args: &[String]) -> Vec<String> {
    let mut out = vec![];
    let mut it = args.iter().peekable();
    if let Some(first) = it.peek()
        && !first.starts_with("--")
    {
        out.extend(["--name".to_string(), it.next().unwrap().clone()]);
    }
    while let Some(a) = it.next() {
        out.push(a.clone());
        if a == "--members"
            && let Some(v) = it.next()
        {
            let list: Vec<&str> = v.split(',').map(|m| m.trim().trim_start_matches('@')).filter(|m| !m.is_empty()).collect();
            out.push(if serde_json::from_str::<Vec<String>>(v).is_ok() { v.clone() } else { serde_json::to_string(&list).unwrap_or_default() });
        }
    }
    out
}

/// Pure: the ID of the channel `key` names in `channel list`'s answer (its ID, or a named channel's name with or
/// without `#`); None when none matches.
fn channel_id_in(list: &serde_json::Value, key: &str) -> Option<String> {
    let channels = list["channels"].as_array()?;
    let by = |f: &dyn Fn(&serde_json::Value) -> bool| channels.iter().find(|c| f(c)).and_then(|c| c["id"].as_str()).map(str::to_string);
    by(&|c| c["id"].as_str() == Some(key)).or_else(|| by(&|c| c["kind"] == "channel" && c["name"].as_str() == Some(key.trim_start_matches('#'))))
}

/// `launchpad channel edit`: the `channel.rename` action, with the channel named by its ID or name.
async fn channel_edit(paths: &Paths, channel: String, name: Option<String>, purpose: Option<String>) -> Result<()> {
    anyhow::ensure!(name.is_some() || purpose.is_some(), "give --name <name>, --purpose <text> or both");
    let list = Client::new(paths)?.json("GET", "/api/channels/agent", None).await?;
    let id = channel_id_in(&list, &channel).with_context(|| format!("no channel {channel:?} in this workspace: see `launchpad channel list`"))?;
    let mut args = vec![id];
    for (flag, v) in [("--name", name), ("--purpose", purpose)] {
        if let Some(v) = v {
            args.extend([flag.to_string(), v]);
        }
    }
    do_action(paths, "channel.rename", &args).await
}

/// `launchpad channel read`: messages with readable times.
async fn channel_read(paths: &Paths, channel: String, since: Option<i64>, limit: u32) -> Result<()> {
    let (since, limit) = (since.map(|s| s.to_string()), limit.to_string());
    let mut pairs = vec![("channel", channel.as_str()), ("limit", limit.as_str())];
    if let Some(s) = &since {
        pairs.push(("since", s.as_str()));
    }
    let mut v = Client::new(paths)?.json("GET", &query("/api/channels/agent/read", &pairs), None).await?;
    for m in v["messages"].as_array_mut().into_iter().flatten() {
        if let Some(obj) = m.as_object_mut()
            && let Some(ms) = obj.remove("created_at").and_then(|t| t.as_i64())
        {
            obj.insert("at".into(), serde_json::Value::String(utc(ms)));
        }
    }
    println!("{}", serde_json::to_string_pretty(&v)?);
    Ok(())
}

#[derive(Subcommand)]
enum ConversationCommand {
    /// Messages of your conversation, oldest first (JSON): after `--since <id>` (a message's `id`), else the newest
    /// `--limit`.
    Read {
        #[arg(long)]
        since: Option<i64>,
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
}

async fn conversation_read(paths: &Paths, since: Option<i64>, limit: u32) -> Result<()> {
    let (since, limit) = (since.map(|s| s.to_string()), limit.to_string());
    let mut pairs = vec![("limit", limit.as_str())];
    if let Some(s) = &since {
        pairs.push(("since", s.as_str()));
    }
    let mut v = Client::new(paths)?.json("GET", &query("/api/context/conversation", &pairs), None).await?;
    for m in v["messages"].as_array_mut().into_iter().flatten() {
        if let Some(obj) = m.as_object_mut()
            && let Some(ms) = obj.get("id").and_then(|t| t.as_i64())
        {
            obj.insert("at".into(), serde_json::Value::String(utc(ms)));
        }
    }
    println!("{}", serde_json::to_string_pretty(&v)?);
    Ok(())
}

#[derive(Subcommand)]
enum BotCommand {
    /// The bot's own notes, in memory/ of its folder ($LAUNCHPAD_BOT/memory, read-only): paths are relative to memory/.
    Memory {
        #[command(subcommand)]
        command: BotMemory,
    },
    /// The scheduled bot messages that wait (`bot message <bot> --text … --in 2h|--at <RFC 3339>`), soonest first
    /// (JSON): those your bot scheduled (any of its threads), else those of your session. Cancel one with
    /// `bot cancel-queued <bot_id> <n>`.
    Scheduled,
    #[command(external_subcommand)]
    Verb(Vec<String>),
}

#[derive(Subcommand)]
enum BotMemory {
    /// List the notes.
    List,
    /// Print a note.
    Read { path: String },
    /// Write a note from stdin (or --file); replaces it.
    Write {
        path: String,
        #[arg(long)]
        file: Option<std::path::PathBuf>,
        #[arg(long, env = "LAUNCHPAD_SESSION_ID", hide_env_values = true)]
        session_id: String,
    },
    /// Add stdin (or --file, or the text arguments) to the end of a note.
    Append {
        path: String,
        text: Vec<String>,
        #[arg(long)]
        file: Option<std::path::PathBuf>,
        #[arg(long, env = "LAUNCHPAD_SESSION_ID", hide_env_values = true)]
        session_id: String,
    },
    /// Remove a note (restorable for 30 days).
    Rm {
        path: String,
        #[arg(long, env = "LAUNCHPAD_SESSION_ID", hide_env_values = true)]
        session_id: String,
    },
}

/// `$LAUNCHPAD_BOT/memory`.
fn bot_memory_dir() -> Result<std::path::PathBuf> {
    std::env::var_os("LAUNCHPAD_BOT").map(|d| std::path::PathBuf::from(d).join("memory")).context("no bot memory here: launchpad bot memory works in a bot's session ($LAUNCHPAD_BOT is not set)")
}

/// A note path: relative to memory/, or a path inside it.
fn bot_memory_path(path: &str) -> Result<String> {
    let p = std::path::Path::new(path);
    if p.is_absolute() {
        let dir = bot_memory_dir()?;
        let rel = p.strip_prefix(&dir).with_context(|| format!("{path} is not in the bot's memory ({})", dir.display()))?;
        return Ok(rel.to_string_lossy().into_owned());
    }
    Ok(path.to_string())
}

async fn bot_memory_command(paths: &Paths, command: BotMemory) -> Result<()> {
    use base64::Engine;
    let b64 = |d: &[u8]| base64::engine::general_purpose::STANDARD.encode(d);
    let (session_id, op) = match command {
        BotMemory::List => {
            let dir = bot_memory_dir()?;
            let files = launchpad_core::plain::walk(&dir);
            if files.is_empty() {
                println!("No note yet.");
            }
            for rel in files {
                let lines = std::fs::read_to_string(dir.join(&rel)).map(|t| t.lines().count()).unwrap_or(0);
                println!("{rel} ({lines} lines)");
            }
            return Ok(());
        }
        BotMemory::Read { path } => {
            let rel = launchpad_core::bot_memory::store_path(&bot_memory_path(&path)?)?;
            let file = bot_memory_dir()?.join(rel.strip_prefix(launchpad_core::bot_memory::DIR).unwrap_or(&rel));
            print!("{}", std::fs::read_to_string(&file).with_context(|| format!("there is no {rel}"))?);
            return Ok(());
        }
        BotMemory::Write { path, file, session_id } => (session_id, serde_json::json!({"op": "write", "path": bot_memory_path(&path)?, "content_b64": b64(&read_input(file.as_deref())?)})),
        BotMemory::Append { path, text, file, session_id } => {
            let text = if !text.is_empty() && file.is_none() { text.join(" ") } else { String::from_utf8(read_input(file.as_deref())?).context("append takes text")? };
            (session_id, serde_json::json!({"op": "append", "path": bot_memory_path(&path)?, "text": text}))
        }
        BotMemory::Rm { path, session_id } => (session_id, serde_json::json!({"op": "remove", "path": bot_memory_path(&path)?})),
    };
    let body = Client::new(paths)?.json("POST", "/api/bot-memory", Some(serde_json::json!({"session_id": session_id, "ops": [op]}))).await?;
    for line in body["lines"].as_array().into_iter().flatten() {
        println!("{}", line.as_str().unwrap_or_default());
    }
    Ok(())
}

fn kb_dir() -> Result<std::path::PathBuf> {
    std::env::var_os("LAUNCHPAD_KB").map(std::path::PathBuf::from).context("no knowledge base here: launchpad kb works in an agent session of a workspace ($LAUNCHPAD_KB is not set)")
}

/// A note path: relative to the knowledge base, or a path inside its folder.
fn kb_path(path: &str) -> Result<String> {
    let p = std::path::Path::new(path);
    if p.is_absolute() {
        let dir = kb_dir()?;
        let rel = p.strip_prefix(&dir).with_context(|| format!("{path} is not in the knowledge base ({})", dir.display()))?;
        return Ok(rel.to_string_lossy().into_owned());
    }
    Ok(path.trim_start_matches("./").to_string())
}

fn read_input(file: Option<&std::path::Path>) -> Result<Vec<u8>> {
    use std::io::Read;
    match file {
        Some(f) => std::fs::read(f).with_context(|| format!("read {}", f.display())),
        None => {
            let mut buf = vec![];
            std::io::stdin().read_to_end(&mut buf)?;
            Ok(buf)
        }
    }
}

async fn kb_command(paths: &Paths, command: Kb) -> Result<()> {
    use base64::Engine;
    let b64 = |d: &[u8]| base64::engine::general_purpose::STANDARD.encode(d);
    let (session_id, op) = match command {
        Kb::Path => {
            println!("{}", kb_dir()?.display());
            return Ok(());
        }
        Kb::Search { words } => {
            let dir = kb_dir()?;
            let mut bodies = vec![];
            for rel in launchpad_core::plain::walk(&dir) {
                if let Ok(text) = std::fs::read_to_string(dir.join(&rel)) {
                    bodies.push((rel, text));
                }
            }
            let hits = launchpad_core::kb::search_text(bodies.iter().map(|(p, t)| (p.clone(), t.as_str())), &words.join(" "), 200);
            if hits.is_empty() {
                println!("No match.");
            }
            for h in hits {
                println!("{}:{}: {}", h.path, h.line, h.text);
            }
            return Ok(());
        }
        Kb::Write { path, file, session_id } => (session_id, serde_json::json!({"op": "write", "path": kb_path(&path)?, "content_b64": b64(&read_input(file.as_deref())?)})),
        Kb::Append { path, text, file, session_id } => {
            let text = if !text.is_empty() && file.is_none() { text.join(" ") } else { String::from_utf8(read_input(file.as_deref())?).context("append takes text")? };
            (session_id, serde_json::json!({"op": "append", "path": kb_path(&path)?, "text": text}))
        }
        Kb::AddMedia { file, to, session_id } => {
            let name = file.file_name().context("name a file")?.to_string_lossy().into_owned();
            let path = match to {
                Some(t) => kb_path(&t)?,
                None => format!("media/{name}"),
            };
            (session_id, serde_json::json!({"op": "write", "path": path, "content_b64": b64(&read_input(Some(&file))?)}))
        }
        Kb::Rm { path, session_id } => (session_id, serde_json::json!({"op": "remove", "path": kb_path(&path)?})),
    };
    let body = Client::new(paths)?.json("POST", "/api/kb", Some(serde_json::json!({"session_id": session_id, "ops": [op]}))).await?;
    for line in body["lines"].as_array().into_iter().flatten() {
        println!("{}", line.as_str().unwrap_or_default());
    }
    Ok(())
}

#[derive(Subcommand)]
enum Assets {
    /// Make `path` of this worktree an asset folder, mapped in `.launchpad/assets.toml` (commit that file).
    Create {
        path: String,
        #[arg(long)]
        session_id: String,
    },
}
async fn assets_command(paths: &Paths, command: Assets) -> Result<()> {
    let Assets::Create { path, session_id } = command;
    let cwd = std::env::current_dir()?.to_string_lossy().into_owned();
    let body = Client::new(paths)?.json("POST", "/api/assets", Some(serde_json::json!({"session_id": session_id, "cwd": cwd, "path": path}))).await?;
    println!("{}", body["message"].as_str().unwrap_or_default());
    Ok(())
}
async fn work_command(paths: &Paths, command: Work) -> Result<()> {
    let (path, body) = match command {
        Work::Finish { session_id } => ("finish", serde_json::json!({"session_id": session_id})),
        Work::AddProject { session_id, project } => ("add-project", serde_json::json!({"session_id": session_id, "project": project})),
    };
    let body = Client::new(paths)?.json("POST", &format!("/api/worktrees/{path}"), Some(body)).await?;
    println!("{}", body["message"].as_str().unwrap_or_default());
    Ok(())
}

#[derive(Subcommand)]
enum Pr {
    /// Create a dedicated local worktree from this checkout's committed HEAD.
    Create {
        #[arg(long)]
        title: Option<String>,
        /// Markdown body; Fixes #N, Closes #N or Resolves #N links issues in the folder repository.
        #[arg(long)]
        body_file: Option<std::path::PathBuf>,
    },
    /// Update the local PR for the current worktree.
    Update {
        #[arg(long)]
        title: Option<String>,
        /// Markdown body; Fixes #N, Closes #N or Resolves #N links issues in the folder repository.
        #[arg(long)]
        body_file: Option<std::path::PathBuf>,
    },
    /// Show the local PR for the current worktree.
    Show,
    /// List local PRs, including published and archived records.
    List,
    /// Publish a Local PR of this bot as a draft pull request on GitHub, at the commit you checked. Only a bot's own
    /// Local PRs.
    Publish {
        /// The Local PR (default: the one of the current worktree).
        #[arg(long)]
        id: Option<String>,
        /// The branch's commit you checked (40 characters); refused when the branch moved since.
        #[arg(long)]
        head: String,
        /// The branch name on GitHub (default: the Local PR's default remote branch).
        #[arg(long)]
        remote_branch: Option<String>,
    },
}
async fn pr_command(paths: &Paths, command: Pr) -> Result<()> {
    let client = Client::new(paths)?;
    let cwd = std::env::current_dir()?.to_string_lossy().to_string();
    let resolve = || async { client.json("GET", &query("/api/predrafts/resolve", &[("cwd", &cwd)]), None).await };
    let id = |p: &serde_json::Value| p["id"].as_str().map(str::to_string).context("pre-draft id");
    let body = match command {
        Pr::List => client.json("GET", "/api/predrafts", None).await?,
        Pr::Create { title, body_file } => client.json("POST", "/api/predrafts", Some(serde_json::json!({"cwd":cwd,"title":title,"body":body_file.map(std::fs::read_to_string).transpose()?}))).await?,
        Pr::Show => {
            let p = resolve().await?;
            client.json("GET", &format!("/api/predrafts/{}", id(&p)?), None).await?
        }
        Pr::Update { title, body_file } => {
            let p = resolve().await?;
            client.json("PATCH", &format!("/api/predrafts/{}", id(&p)?), Some(serde_json::json!({"title":title,"body":body_file.map(std::fs::read_to_string).transpose()?}))).await?
        }
        Pr::Publish { id: pr, head, remote_branch } => {
            let pr = match pr {
                Some(pr) => pr,
                None => id(&resolve().await?)?,
            };
            let remote_branch = match remote_branch {
                Some(b) => b,
                None => client.json("GET", &format!("/api/predrafts/{pr}"), None).await?["default_remote_branch"].as_str().context("default remote branch")?.to_string(),
            };
            client.json("POST", &format!("/api/predrafts/{pr}/publish"), Some(serde_json::json!({"remote_branch": remote_branch, "draft": true, "head": head}))).await?
        }
    };
    println!("{}", serde_json::to_string_pretty(&body)?);
    Ok(())
}

fn main() -> Result<()> {
    // The network capture of contained sessions forks and execs: it runs before any thread starts.
    #[cfg(target_os = "linux")]
    {
        let args: Vec<String> = std::env::args().collect();
        match args.get(1).map(String::as_str) {
            Some("netcap") => netcap::main(&args[2..]),
            Some("netcap-probe") => netcap::probe(&args[2..]),
            _ => {}
        }
    }
    match tokio::runtime::Builder::new_multi_thread().enable_all().build()?.block_on(run()) {
        Err(e) if e.chain().any(|c| c.is::<Retryable>()) => {
            eprintln!("Error: {e:#}\nThis is temporary: run the same command again later (exit code {RETRY_EXIT}).");
            std::process::exit(RETRY_EXIT)
        }
        r => r,
    }
}

async fn run() -> Result<()> {
    launchpad_core::set_build(env!("LAUNCHPAD_COMMIT"), env!("LAUNCHPAD_SOURCE"));
    launchpad_harness::install();
    launchpad_core::ext::install(Box::new(extensions::Extensions));
    let paths = Paths::from_env();
    let cli = Cli::parse();
    match cli.command {
        Command::Link {
            repo,
            number,
            comment_url,
            session_id,
        } => {
            let value = Client::new(&paths)?.json("POST", "/api/link", Some(serde_json::json!({"repo":repo,"number":number,"comment_url":comment_url,"session_id":session_id}))).await?;
            println!("{value}");
            Ok(())
        }
        Command::Note { repo, number, body_file, suggest_file, reply_to, actions, reply_to_note, session_id } => {
            let body = std::fs::read_to_string(&body_file).with_context(|| format!("reading {}", body_file.display()))?;
            let suggest = match &suggest_file {
                Some(f) => Some(std::fs::read_to_string(f).with_context(|| format!("reading {}", f.display()))?),
                None => None,
            };
            let value = Client::new(&paths)?.json("POST", "/api/notes", Some(serde_json::json!({"repo":repo,"number":number,"body":body,"suggest":suggest,"suggest_reply_to":reply_to,"suggest_actions":actions.to_json(),"reply_to":reply_to_note,"session_id":session_id}))).await?;
            println!("{value}");
            Ok(())
        }
        Command::Notes { repo, number } => {
            let value = Client::new(&paths)?.json("GET", &query("/api/context/notes", &[("repo", &repo), ("number", &number.to_string())]), None).await?;
            println!("{}", serde_json::to_string_pretty(&value)?);
            Ok(())
        }
        Command::Sleep { delay, until, agents, state, quota, timeout, note, session_id } => {
            use launchpad_automation::schedules::{AgentState, parse_minutes, parse_quota, parse_until};
            let minutes = delay.as_deref().map(parse_minutes).transpose()?;
            let at = until.as_deref().map(parse_until).transpose()?;
            let timeout_minutes = timeout.as_deref().map(parse_minutes).transpose()?;
            let mut conditions = Vec::new();
            let agents: Vec<String> = agents.into_iter().map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect();
            if !agents.is_empty() {
                let states = state.iter().map(|s| AgentState::parse(s)).collect::<Result<Vec<_>>>()?;
                conditions.push(serde_json::json!({"kind": "agents", "session_ids": agents, "states": states}));
            }
            if let Some(q) = &quota {
                conditions.push(parse_quota(q)?);
            }
            let value = Client::new(&paths)?.json("POST", "/api/sleep", Some(serde_json::json!({"session_id": session_id, "minutes": minutes, "at": at, "conditions": conditions, "timeout_minutes": timeout_minutes, "note": note}))).await?;
            let next = if value["already_met"].as_bool() == Some(true) {
                "end your turn now: the conditions already hold, so Launchpad wakes this session as soon as this turn ends"
            } else {
                "end your turn now: Launchpad wakes this session once the conditions hold, in this conversation"
            };
            println!("{}", serde_json::json!({"sleeping_until": value["ready_at"], "waits_for": value["waits"], "timeout": value["deadline"], "already_met": value["already_met"], "schedule_id": value["id"], "next": next}));
            Ok(())
        }
        Command::Suggest { command } => suggest_command(&paths, command).await,
        Command::Ci { command: Ci::Approve { repo, number, head, session_id } } => {
            let value = Client::new(&paths)?.json("POST", "/api/ci/approve", Some(serde_json::json!({"repo": repo, "number": number, "head": head, "session_id": session_id}))).await?;
            println!("{value}");
            Ok(())
        }
        Command::Send { connection, body_file, title, session_id } => {
            let body = std::fs::read_to_string(&body_file).with_context(|| format!("read {}", body_file.display()))?;
            let session_id = session_id.or_else(|| std::env::var("LAUNCHPAD_SESSION_ID").ok()).context("pass --session-id <your session id>")?;
            let result = Client::new(&paths)?.json("POST", "/api/connections/send", Some(serde_json::json!({"connection": connection, "body": body, "title": title, "session_id": session_id}))).await?;
            println!("{}", result["message"].as_str().unwrap_or("Done."));
            Ok(())
        }
        Command::Pr { command } => pr_command(&paths, command).await,
        Command::Work { command } => work_command(&paths, command).await,
        Command::Assets { command } => assets_command(&paths, command).await,
        Command::Kb { command } => kb_command(&paths, command).await,
        Command::Bot { command: BotCommand::Memory { command } } => bot_memory_command(&paths, command).await,
        Command::Bot { command: BotCommand::Scheduled } => context_command(&paths, "scheduled-messages", None).await,
        Command::Bot { command: BotCommand::Verb(words) } => {
            let [verb, args @ ..] = words.as_slice() else { anyhow::bail!("name a bot action: see `launchpad --help`") };
            do_action(&paths, &format!("bot.{verb}"), args).await
        }
        Command::Folders { command: ListOnly::List { workspace } } => context_command(&paths, "folders", workspace).await,
        Command::Bots { command: ListOnly::List { workspace } } => context_command(&paths, "bots", workspace).await,
        Command::Profiles { command: ListOnly::List { .. } } => context_command(&paths, "profiles", None).await,
        Command::Presets { command: ListOnly::List { .. } } => context_command(&paths, "presets", None).await,
        Command::Egress { command: ListOnly::List { .. } } => context_command(&paths, "egress", None).await,
        Command::Schedules { command: ListOnly::List { workspace } } => context_command(&paths, "schedules", workspace).await,
        Command::Sessions { command: Sessions::List { workspace } } => context_command(&paths, "sessions", workspace).await,
        Command::Sessions { command: Sessions::Show { session_id } } => context_command(&paths, &format!("sessions/{}", url_segment(&session_id)), None).await,
        Command::Proposals { command: ListOnly::List { workspace } } => {
            let path = match workspace { Some(w) => query("/api/proposals", &[("workspace", &w)]), None => "/api/proposals".into() };
            let proposals = Client::new(&paths)?.json("GET", &path, None).await?;
            println!("{}", serde_json::to_string_pretty(&proposals)?);
            Ok(())
        }
        Command::Logs { runner, home, local, session, since, level, grep, limit, json } => logs_command(&paths, runner, home, local, session, since, level, grep, limit, json).await,
        Command::Health => list_command(&paths, "/api/context/health", None).await,
        Command::Failures { since } => {
            let path = match since { Some(s) => query("/api/context/failures", &[("since", &s)]), None => "/api/context/failures".into() };
            list_command(&paths, &path, None).await
        }
        Command::Runners { command: ListOnly::List { .. } } => list_command(&paths, "/api/context/runners", None).await,
        Command::Burst { command: Burst::Runs { session, limit } } => {
            let limit = limit.map(|l| l.to_string());
            let mut pairs = vec![];
            if let Some(s) = &session { pairs.push(("session", s.as_str())); }
            if let Some(l) = &limit { pairs.push(("limit", l.as_str())); }
            list_command(&paths, &query("/api/context/burst", &pairs), None).await
        }
        Command::Burst { command: Burst::Job { runner_id } } => list_command(&paths, &format!("/api/context/burst/{}/job", url_segment(&runner_id)), None).await,
        Command::Home => list_command(&paths, "/api/context/home", None).await,
        Command::Tools { command: ListOnly::List { .. } } => list_command(&paths, "/api/context/tools", None).await,
        Command::Connections { command: ListOnly::List { .. } } => list_command(&paths, "/api/context/connections", None).await,
        Command::Conversation { command: ConversationCommand::Read { since, limit } } => conversation_read(&paths, since, limit).await,
        Command::Channel { command: ChannelCommand::List } => list_command(&paths, "/api/channels/agent", None).await,
        Command::Channel { command: ChannelCommand::Read { channel, since, limit } } => channel_read(&paths, channel, since, limit).await,
        Command::Channel { command: ChannelCommand::Post { channel, post } } => channel_post(&paths, channel, post).await,
        Command::Channel { command: ChannelCommand::Files { channel, message, dir } } => channel_files(&paths, channel, message, dir).await,
        Command::Channel { command: ChannelCommand::Join { channel, mode, note, session_id } } => {
            let mut body = serde_json::json!({"kind": "channel_join", "channel": channel});
            if let Some(m) = mode {
                body["mode"] = serde_json::Value::String(m);
            }
            let created = Client::new(&paths)?.json("POST", "/api/proposals", Some(serde_json::json!({"title": format!("Join #{}", channel.trim_start_matches('#')), "note": note, "body": body, "session_id": session_id}))).await?;
            println!("Card {} saved: {} (state: {}). The owner accepts it.", created["id"].as_str().unwrap_or("?"), created["title"].as_str().unwrap_or(""), created["state"].as_str().unwrap_or("unknown"));
            Ok(())
        }
        Command::Channel { command: ChannelCommand::Edit { channel, name, purpose } } => channel_edit(&paths, channel, name, purpose).await,
        Command::Channel { command: ChannelCommand::Verb(words) } => {
            let [verb, args @ ..] = words.as_slice() else { anyhow::bail!("name a channel command: see `launchpad channel --help`") };
            let args = if verb == "create" { channel_create_args(args) } else { args.to_vec() };
            do_action(&paths, &format!("channel.{verb}"), &args).await
        }
        Command::Dm { bot, post } => channel_post(&paths, format!("@{}", bot.trim_start_matches('@')), post).await,
        Command::Oracle { workspace, command } => {
            let code = match oracle::command(&paths, workspace, command).await {
                Ok(code) => code,
                Err(e) => {
                    eprintln!("Error: {e:#}");
                    oracle::ERROR_EXIT
                }
            };
            std::process::exit(code)
        }
        Command::Folio { workspace, command } => {
            let code = match folio::command(&paths, workspace, command).await {
                Ok(code) => code,
                Err(e) => {
                    eprintln!("Error: {e:#}");
                    folio::ERROR_EXIT
                }
            };
            std::process::exit(code)
        }
        Command::Hops => {
            let v = Client::new(&paths)?.json("GET", "/api/context/hops", None).await?;
            println!("{}", v["hops"].as_u64().context("Home did not return a hop count")?);
            Ok(())
        }
        Command::Slack { profile, args } => {
            let code = slack::run(&Client::new(&paths)?, profile, args).await?;
            std::process::exit(code)
        }
        Command::Today { workspace } => list_command(&paths, "/api/today", workspace).await,
        Command::Activity { command: ListOnly::List { workspace } } => list_command(&paths, "/api/activity", workspace).await,
        Command::Propose { kind, title, note, replaces, spec_file, session_id } => {
            let text = std::fs::read_to_string(&spec_file).with_context(|| format!("read {}", spec_file.display()))?;
            let mut body: serde_json::Value = serde_json::from_str(&text).context("the spec file is not valid JSON")?;
            anyhow::ensure!(body.is_object(), "the spec file must hold a JSON object");
            body["kind"] = serde_json::Value::String(kind.replace('-', "_"));
            let session_id = session_id.or_else(|| std::env::var("LAUNCHPAD_SESSION_ID").ok());
            let created = Client::new(&paths)?.json("POST", "/api/proposals", Some(serde_json::json!({"title": title, "note": note, "replaces": replaces, "body": body, "session_id": session_id}))).await;
            // An oracle card the pre-check refuses is not filed: its errors, exit 3 like `oracle save` and `oracle test`.
            let created = match created {
                Err(e) if kind == "oracle" => {
                    eprintln!("Error: {e:#}");
                    std::process::exit(oracle::ERROR_EXIT)
                }
                r => r?,
            };
            if kind == "oracle" {
                oracle::print_warnings(&created["body"]["warnings"]);
            }
            println!("Proposal {} saved (state: {}).", created["id"].as_str().unwrap_or("?"), created["state"].as_str().unwrap_or("unknown"));
            if let Some(error) = created["error"].as_str() { println!("Error: {error}"); }
            Ok(())
        }
        Command::Api { method, path, body_file } => {
            let method = method.to_ascii_uppercase();
            anyhow::ensure!(matches!(method.as_str(), "GET" | "POST" | "PUT" | "PATCH" | "DELETE"), "the method is GET, POST, PUT, PATCH or DELETE");
            anyhow::ensure!(path.starts_with("/api/"), "the path starts with /api/");
            let body = match body_file {
                Some(f) => Some(serde_json::from_str(&std::fs::read_to_string(&f).with_context(|| format!("read {}", f.display()))?).context("the body file is not valid JSON")?),
                None => None,
            };
            let result = Client::new(&paths)?.json(&method, &path, body).await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(())
        }
        Command::Do { action, args } => do_action(&paths, &action, &args).await,
        Command::Verb(words) => {
            let [group, verb, args @ ..] = words.as_slice() else { anyhow::bail!("unknown command {:?}: see `launchpad --help`", words.join(" ")) };
            do_action(&paths, &format!("{group}.{verb}"), args).await
        }
        Command::Topics { verb, id, workspace } => {
            anyhow::ensure!(verb == "list" || verb == "show", "Use topics list or topics show <id or name>");
            let mut pairs = Vec::new();
            if let Some(ref ws) = workspace { pairs.push(("workspace", ws.as_str())); }
            if let Some(ref id) = id { pairs.push(("id", id.as_str())); }
            let value = Client::new(&paths)?.json("GET", &query("/api/context/topics", &pairs), None).await?;
            println!("{}", serde_json::to_string_pretty(&value)?); Ok(())
        }
        Command::Trace { session_id, through } => {
            let mut path = format!("/api/sessions/{}/trace", url_segment(&session_id));
            if let Some(through) = through {
                path = query(&path, &[("through", &through.to_string())]);
            }
            let (status, text) = Client::new(&paths)?.send("GET", &path, None).await?;
            if !(200..300).contains(&status) {
                let body: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                anyhow::bail!("{}", body["error"].as_str().unwrap_or("request failed"));
            }
            println!("{text}");
            Ok(())
        }
        Command::ProxyCa => {
            println!("{}", launchpad_core::proxy::ca_certificate(&paths)?.display());
            Ok(())
        }
        Command::Daemon { role } => {
            init_tracing();
            let role = launchpad_core::role::Role::parse(&role)?;
            if role == launchpad_core::role::Role::Runner {
                launchpad_core::role::ensure_agent_os()?;
            }
            // A runner gets its own folders and port (`LAUNCHPAD_PORT` overrides the port).
            let paths = if role == paths.role { paths } else { Paths::for_role(paths.app_env, role) };
            if let Err(e) = launchpad_http::logs::init(&paths.data_dir) {
                tracing::warn!("recent log: {e}");
            }
            tracing::info!("env {}, role {}, data in {}", paths.app_env.name(), role.name(), paths.data_dir.display());
            let app = launchpad_core::App::start(paths).await?;
            extensions::start(&app).await?;
            launchpad_server::serve(app.clone(), launchpad_api::router()).await?;
            if let Err(e) = launchpad_automation::failures::record_stop(&app).await {
                tracing::warn!("daemon stop record: {e:#}");
            }
            // A runner that just installed Home's build starts it in this process.
            launchpad_core::self_update::restart_if_updated();
            Ok(())
        }
        Command::Connect { url, name } => {
            launchpad_core::role::ensure_agent_os()?;
            let runner = Paths::for_role(paths.app_env, launchpad_core::role::Role::Runner);
            launchpad_server::connect(&runner, &url, name).await
        }
        Command::Status => {
            let body = Client::new(&paths)?.json("GET", "/api/status", None).await?;
            println!("{}", serde_json::to_string_pretty(&body)?);
            Ok(())
        }
        Command::Url | Command::Pair => {
            let res = reqwest::Client::new().post(format!("{}/api/devices/pair", paths.base_url()))
                .bearer_auth(paths.read_token()?).send().await.context("is the daemon running?")?;
            let body: serde_json::Value = res.error_for_status()?.json().await?;
            println!("{}", body["url"].as_str().context("missing pairing URL")?);
            if let Some(warning) = body["warning"].as_str() {
                eprintln!("Warning: {warning}");
            }
            if matches!(cli.command, Command::Pair) {
                println!("\n{}\nSingle-use link; expires in five minutes.", body["terminal"].as_str().unwrap_or(""));
            }
            Ok(())
        }
        Command::GenTypes { out } => extensions::gen_types(&out),
    }
}

/// The request of an action from its arguments: route ids in order, `--<field> <value>`, `--body-file <file>`.
fn action_request(action: &str, args: &[String]) -> Result<(&'static launchpad_core::actions::Action, String, Option<serde_json::Value>)> {
    use launchpad_core::actions::{self, ActionAccess, FieldKind};
    let a = actions::get(action).with_context(|| format!("no action {action:?}: the Launchpad CLI reference lists them"))?;
    anyhow::ensure!(a.access == ActionAccess::Admin, "{} is the owner's click only: ask the owner to use the Launchpad window", a.name);
    let (mut ids, mut fields, mut body_file) = (vec![], serde_json::Map::new(), None);
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let Some(name) = arg.strip_prefix("--") else { ids.push(arg.as_str()); continue };
        let value = it.next().with_context(|| format!("--{name} needs a value"))?;
        if name == "body-file" { body_file = Some(value); continue; }
        let field = a.input.iter().find(|f| f.name == name || f.name.replace('_', "-") == name).with_context(|| format!("{} has no --{name} option: {}", a.name, actions::usage("launchpad", a)))?;
        let v = match field.kind {
            FieldKind::Text => serde_json::Value::String(value.clone()),
            FieldKind::Bool => serde_json::Value::Bool(value.parse().with_context(|| format!("--{name} is true or false"))?),
            FieldKind::Json => serde_json::from_str(value).with_context(|| format!("--{name} takes JSON"))?,
        };
        fields.insert(field.name.into(), v);
    }
    let path = actions::path(a, &ids)?;
    let mut body = match body_file {
        Some(f) => serde_json::from_str(&std::fs::read_to_string(f).with_context(|| format!("read {f}"))?).context("the body file is not valid JSON")?,
        None => {
            anyhow::ensure!(a.body_file.is_none(), "{} takes --body-file <file.json> ({})", a.name, a.body_file.unwrap_or_default());
            for f in a.input.iter().filter(|f| !f.optional) {
                anyhow::ensure!(fields.contains_key(f.name), "{} needs --{}: {}", a.name, f.name, actions::usage("launchpad", a));
            }
            serde_json::Value::Object(Default::default())
        }
    };
    let obj = body.as_object_mut().context("the body file must hold a JSON object")?;
    obj.extend(fields);
    if let Some(fixed) = a.fixed {
        obj.extend(serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(fixed).expect("valid fixed body"));
    }
    let body = (a.method != "DELETE").then_some(body);
    Ok((a, path, body))
}

async fn do_action(paths: &Paths, action: &str, args: &[String]) -> Result<()> {
    let (a, path, body) = action_request(action, args)?;
    let result = Client::new(paths)?.json(a.method, &path, body).await?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

fn url_segment(s: &str) -> String {
    let mut url = reqwest::Url::parse("http://x/").expect("valid url");
    url.path_segments_mut().expect("base").push(s);
    url.path().trim_start_matches('/').to_string()
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let journald = std::env::var_os("JOURNAL_STREAM").is_some();
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    // journald adds its own timestamps.
    let plain = journald.then(|| tracing_subscriber::fmt::layer().with_target(false).without_time().with_ansi(false));
    let timed = (!journald).then(|| tracing_subscriber::fmt::layer().with_target(false));
    tracing_subscriber::registry().with(filter).with(plain).with(timed).with(RecentLog).init();
}

/// Copies every log line to the daemon's recent log (`launchpad_http::logs`, redacted), which `launchpad logs`
/// reads.
struct RecentLog;

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for RecentLog {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        struct Text(String);
        impl tracing::field::Visit for Text {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                use std::fmt::Write;
                if field.name() == "message" {
                    let _ = write!(self.0, "{}{value:?}", if self.0.is_empty() { "" } else { " " });
                } else {
                    let _ = write!(self.0, "{}{}={value:?}", if self.0.is_empty() { "" } else { " " }, field.name());
                }
            }
            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                if field.name() == "message" {
                    if !self.0.is_empty() { self.0.push(' '); }
                    self.0.push_str(value);
                } else {
                    self.record_debug(field, &value);
                }
            }
        }
        let mut text = Text(String::new());
        event.record(&mut text);
        let meta = event.metadata();
        launchpad_http::logs::push(meta.level().as_str(), meta.target(), &text.0);
    }
}

#[cfg(test)]
mod proposal_tests {
    use super::*;

    #[test]
    fn channel_files_get_a_type_and_a_safe_local_name() {
        assert_eq!(file_mime(std::path::Path::new("shot.PNG")), "image/png");
        assert_eq!(file_mime(std::path::Path::new("notes.md")), "text/markdown");
        assert_eq!(file_mime(std::path::Path::new("logo.svg")), "text/plain");
        assert_eq!(file_mime(std::path::Path::new("archive")), "application/octet-stream");
        let mut taken = std::collections::HashSet::new();
        assert_eq!(download_name("../../etc/passwd", 0, &taken), "passwd");
        assert_eq!(download_name("my report (v2).pdf", 1, &taken), "my_report__v2_.pdf");
        assert_eq!(download_name("..", 2, &taken), "file-2");
        taken.insert("a.txt".to_string());
        assert_eq!(download_name("a.txt", 3, &taken), "3-a.txt");
    }

    #[test]
    fn proposal_cli_accepts_replacement_without_a_note_and_lists_by_workspace() {
        let cli = Cli::try_parse_from(["launchpad", "propose", "launch", "--title", "Refined", "--spec-file", "spec.json", "--replaces", "old-id"]).unwrap();
        assert!(matches!(cli.command, Command::Propose { replaces: Some(id), note: None, .. } if id == "old-id"));
        let cli = Cli::try_parse_from(["launchpad", "propose", "status", "--title", "New", "--spec-file", "spec.json"]).unwrap();
        assert!(matches!(cli.command, Command::Propose { replaces: None, .. }));
        let cli = Cli::try_parse_from(["launchpad", "propose", "preset", "--title", "Preset", "--spec-file", "spec.json"]).unwrap();
        assert!(matches!(cli.command, Command::Propose { kind, .. } if kind == "preset"));
        assert!(Cli::try_parse_from(["launchpad", "propose", "delete", "--title", "X", "--spec-file", "spec.json"]).is_err());
        let cli = Cli::try_parse_from(["launchpad", "logs", "--session", "s1", "--since", "2h"]).unwrap();
        assert!(matches!(cli.command, Command::Logs { session: Some(s), home: false, .. } if s == "s1"));
        assert!(Cli::try_parse_from(["launchpad", "logs", "--home", "--runner", "r"]).is_err());
        assert!(matches!(Cli::try_parse_from(["launchpad", "burst", "job", "r1"]).unwrap().command, Command::Burst { command: Burst::Job { .. } }));
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc(1_762_686_245_000), "2025-11-09T11:04:05Z");
        // `bot memory` is a command; other `bot` words stay the short verbs of bot actions.
        assert!(matches!(Cli::try_parse_from(["launchpad", "bot", "memory", "write", "notes.md", "--session-id", "s1"]).unwrap().command, Command::Bot { command: BotCommand::Memory { command: BotMemory::Write { .. } } }));
        assert!(matches!(Cli::try_parse_from(["launchpad", "bot", "pause", "b1", "--paused", "true"]).unwrap().command, Command::Bot { command: BotCommand::Verb(w) } if w == ["pause", "b1", "--paused", "true"]));
        // A scheduled bot message: `bot message` with --in or --at, listed by `bot scheduled`.
        assert!(matches!(Cli::try_parse_from(["launchpad", "bot", "scheduled"]).unwrap().command, Command::Bot { command: BotCommand::Scheduled }));
        let args: Vec<String> = ["b1", "--text", "check CI", "--in", "2h"].map(String::from).to_vec();
        let (_, path, body) = action_request("bot.message", &args).unwrap();
        assert_eq!((path.as_str(), body), ("/api/bots/b1/message", Some(serde_json::json!({"text": "check CI", "in": "2h"}))));
        // The commands a delivered channel message names work as written.
        let cli = Cli::try_parse_from(["launchpad", "channel", "read", "@release", "--since", "38"]).unwrap();
        assert!(matches!(cli.command, Command::Channel { command: ChannelCommand::Read { channel, since: Some(38), limit: 50 } } if channel == "@release"));
        let cli = Cli::try_parse_from(["launchpad", "conversation", "read", "--since", "1700000000000"]).unwrap();
        assert!(matches!(cli.command, Command::Conversation { command: ConversationCommand::Read { since: Some(1_700_000_000_000), limit: 50 } }));
        let cli = Cli::try_parse_from(["launchpad", "dm", "release", "--body-file", "m.md", "--session-id", "s1"]).unwrap();
        assert!(matches!(cli.command, Command::Dm { bot, post: ChannelPostArgs { ask_owner: false, .. } } if bot == "release"));
        let cli = Cli::try_parse_from(["launchpad", "channel", "post", "release-train", "--body-file", "m.md", "--reply-to", "4", "--ask-owner", "--session-id", "s1"]).unwrap();
        assert!(matches!(cli.command, Command::Channel { command: ChannelCommand::Post { post: ChannelPostArgs { reply_to: Some(4), ask_owner: true, .. }, .. } }));
        assert!(matches!(Cli::try_parse_from(["launchpad", "channel", "add-member", "c1", "b1", "--mode", "all"]).unwrap().command, Command::Channel { command: ChannelCommand::Verb(_) }));
        let cli = Cli::try_parse_from(["launchpad", "channel", "edit", "#release-train", "--purpose", "Ship it"]).unwrap();
        assert!(matches!(cli.command, Command::Channel { command: ChannelCommand::Edit { channel, name: None, purpose: Some(p) } } if channel == "#release-train" && p == "Ship it"));
        let list = serde_json::json!({"channels": [{"id": "c1", "kind": "channel", "name": "release-train"}, {"id": "d1", "kind": "dm", "name": "@docs"}]});
        assert_eq!(channel_id_in(&list, "release-train").as_deref(), Some("c1"));
        assert_eq!(channel_id_in(&list, "#release-train").as_deref(), Some("c1"));
        assert_eq!(channel_id_in(&list, "c1").as_deref(), Some("c1"));
        assert_eq!(channel_id_in(&list, "@docs"), None);
        assert_eq!(channel_id_in(&list, "nope"), None);
        // Files: --attach and --forward repeat, and replace --body-file.
        let cli = Cli::try_parse_from(["launchpad", "dm", "@release", "--attach", "a.pdf", "--attach", "/tmp/b.png", "--forward", "41", "--session-id", "s1"]).unwrap();
        assert!(matches!(cli.command, Command::Dm { post: ChannelPostArgs { body_file: None, attach, forward, .. }, .. } if attach.len() == 2 && forward == [41]));
        let cli = Cli::try_parse_from(["launchpad", "channel", "files", "release-train", "41", "--dir", "out"]).unwrap();
        assert!(matches!(cli.command, Command::Channel { command: ChannelCommand::Files { channel, message: 41, dir: Some(_) } } if channel == "release-train"));
        assert!(matches!(Cli::try_parse_from(["launchpad", "channel", "join", "#release-train", "--mode", "all", "--session-id", "s1"]).unwrap().command, Command::Channel { command: ChannelCommand::Join { channel, mode: Some(m), .. } } if channel == "#release-train" && m == "all"));
        assert!(matches!(Cli::try_parse_from(["launchpad", "propose", "channel", "--title", "New channel", "--spec-file", "c.json"]).unwrap().command, Command::Propose { kind, .. } if kind == "channel"));
        let args = |a: &[&str]| channel_create_args(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(args(&["triage", "--purpose", "Sort issues", "--members", "a, @b"]), ["--name", "triage", "--purpose", "Sort issues", "--members", r#"["a","b"]"#]);
        assert_eq!(args(&["--name", "x", "--members", r#"["a"]"#]), ["--name", "x", "--members", r#"["a"]"#]);
        let cli = Cli::try_parse_from(["launchpad", "proposals", "list", "--workspace", "Work"]).unwrap();
        assert!(matches!(cli.command, Command::Proposals { command: ListOnly::List { workspace: Some(w) } } if w == "Work"));
    }
}

#[cfg(test)]
mod link_cli_tests {
    use super::*;
    #[test]
    fn link_requires_item_and_session() {
        assert!(
            Cli::try_parse_from([
                "launchpad",
                "link",
                "--repo",
                "o/r",
                "--number",
                "12",
                "--session-id",
                "s",
                "--comment-url",
                "https://github.com/o/r/issues/12#issuecomment-1"
            ])
            .is_ok()
        );
        assert!(Cli::try_parse_from(["launchpad", "link", "--repo", "o/r"]).is_err());
    }
}

#[cfg(test)]
mod close_cli_tests {
    use super::*;
    use serde_json::json;
    fn actions_of(args: &[&str]) -> Vec<serde_json::Value> {
        let mut all = vec!["launchpad", "suggest", "comment", "--repo", "o/r", "--number", "3", "--body-file", "c.md"];
        all.extend_from_slice(args);
        match Cli::try_parse_from(all).unwrap().command {
            Command::Suggest { command: Suggest::Comment(q) } => q.actions.to_json(),
            _ => unreachable!(),
        }
    }
    #[test]
    fn close_flags_take_a_github_comment() {
        assert_eq!(actions_of(&["--close"]), vec![json!({"kind": "close", "reason": null})]);
        assert!(actions_of(&[]).is_empty());
        // A Local PR has nothing to close (refused before the call).
        assert!(Cli::try_parse_from(["launchpad", "suggest", "comment", "--predraft", "p", "--body-file", "c.md", "--close"]).is_ok_and(|c| matches!(c.command, Command::Suggest { command: Suggest::Comment(SuggestArgs { predraft: Some(_), .. }) })));
        let cli = Cli::try_parse_from(["launchpad", "note", "--repo", "o/r", "--number", "3", "--body-file", "n.md", "--suggest-file", "c.md", "--close", "--session-id", "s"]).unwrap();
        assert!(matches!(cli.command, Command::Note { suggest_file: Some(_), .. }));
        // A close without a comment: an action-only suggestion under the note.
        let cli = Cli::try_parse_from(["launchpad", "note", "--repo", "o/r", "--number", "3", "--body-file", "n.md", "--close", "not-planned", "--session-id", "s"]).unwrap();
        assert!(matches!(&cli.command, Command::Note { suggest_file: None, actions, .. } if actions.to_json() == vec![json!({"kind": "close", "reason": "not_planned"})]));
    }
    #[test]
    fn action_flags_become_actions() {
        assert_eq!(actions_of(&["--close", "completed", "--session-id", "s"]), vec![json!({"kind": "close", "reason": "completed"})]);
        assert_eq!(actions_of(&["--duplicate-of", "12"]), vec![json!({"kind": "close", "reason": "duplicate", "duplicate_of": 12})]);
        assert_eq!(
            actions_of(&["--ready-for-review", "--request-reviewers", "ana,@bo", "--request-reviewers", "org/core", "--update-branch"]),
            vec![json!({"kind": "update_branch"}), json!({"kind": "ready_for_review"}), json!({"kind": "request_reviewers", "reviewers": ["ana", "bo"], "team_reviewers": ["core"]})]
        );
        assert_eq!(actions_of(&["--reopen", "--convert-to-draft"]), vec![json!({"kind": "reopen"}), json!({"kind": "convert_to_draft"})]);
        for bad in [&["--close", "--reopen"][..], &["--close", "--duplicate-of", "2"], &["--ready-for-review", "--convert-to-draft"], &["--close", "maybe"]] {
            let mut all = vec!["launchpad", "suggest", "comment", "--repo", "o/r", "--number", "3", "--body-file", "c.md"];
            all.extend_from_slice(bad);
            assert!(Cli::try_parse_from(all).is_err(), "{bad:?}");
        }
        let cli = Cli::try_parse_from(["launchpad", "suggest", "action", "--repo", "o/r", "--number", "3", "--close", "not-planned"]).unwrap();
        assert!(matches!(&cli.command, Command::Suggest { command: Suggest::Action(q) } if q.actions.to_json() == vec![json!({"kind": "close", "reason": "not_planned"})]));
        // No body file for an action-only suggestion.
        assert!(Cli::try_parse_from(["launchpad", "suggest", "action", "--repo", "o/r", "--number", "3", "--reopen", "--body-file", "x.md"]).is_err());
    }
    #[test]
    fn ci_approve_needs_the_inspected_head() {
        let cli = Cli::try_parse_from(["launchpad", "ci", "approve", "--repo", "o/r", "--number", "3", "--head", "abc", "--session-id", "s"]).unwrap();
        assert!(matches!(cli.command, Command::Ci { command: Ci::Approve { number: 3, .. } }));
        assert!(Cli::try_parse_from(["launchpad", "ci", "approve", "--repo", "o/r", "--number", "3", "--session-id", "s"]).is_err());
    }
}

#[cfg(test)]
mod slack_cli_tests {
    use super::*;
    #[test]
    fn slack_passes_slacklines_arguments_through() {
        let parse = |a: &[&str]| match Cli::try_parse_from(a).unwrap().command {
            Command::Slack { profile, args } => (profile, args),
            _ => panic!("not slack"),
        };
        let (p, a) = parse(&["launchpad", "slack", "--profile", "user-read", "--json", "search", "messages", "-x deploy", "--limit", "5"]);
        assert_eq!((p.as_deref(), a.as_slice()), (Some("user-read"), ["--json", "search", "messages", "-x deploy", "--limit", "5"].map(String::from).as_slice()));
        let (p, a) = parse(&["launchpad", "slack", "--help"]);
        assert_eq!((p, a), (None, vec!["--help".to_string()]));
        let (p, a) = parse(&["launchpad", "slack", "users", "list", "--profile", "bot"]);
        assert_eq!((p, a.len()), (None, 4));
    }
}

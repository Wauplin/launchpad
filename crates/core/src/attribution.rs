//! The attribution line Launchpad adds to what agents publish: pull request and issue bodies, GitHub comments
//! and reviews, Local PRs, Suggestions and Outbox messages. One line, in italics, always last:
//! `_Written by <name> via [Launchpad](…)_`, naming the harness for a one-time agent or the bot for a bot's
//! session. Bots find their own past comments by it.
//!
//! On GitHub the line also says who triggered the publication (`Origin`), decided by Home from where the write
//! comes from, never from the text: `, posted automatically` for an agent's or bot's own write (its `gh` and API
//! calls through the proxy, `launchpad pr publish`, a launch's When done = Draft PR or Open PR) and `, posted
//! manually` for the owner's click (Post of a Suggestion, Publish of a Local PR, the extension's Insert that the
//! owner submits with GitHub's button). An owner asking an agent to post still counts as automatic.
//!
//! Signing is idempotent: a body that already ends with an attribution line (italic or plain, or Claude Code's
//! default line) gets it replaced, never doubled.

use serde_json::Value;

use crate::App;
use crate::model::Session;

pub const URL: &str = "https://github.com/Wauplin/launchpad";
const LINK: &str = "[Launchpad](https://github.com/Wauplin/launchpad)";
/// Claude Code's own pull request line, replaced by Launchpad's.
const CLAUDE_DEFAULT: &str = "🤖 Generated with [Claude Code](https://claude.com/claude-code)";

/// The line in Markdown: `bot` for a bot's session, else the harness's display name.
pub fn text(bot: Option<&str>, harness: &str) -> String {
    match bot.map(str::trim).filter(|b| !b.is_empty()) {
        Some(b) => format!("_Written by {b} via {LINK}_"),
        None => format!("_Written by {harness} via {LINK}_"),
    }
}

/// The session's line.
pub fn line(s: &Session) -> String {
    text(s.launch_spec.bot.as_ref().map(|b| b.name.as_str()), s.harness.display())
}

/// The line of a session this machine knows; None for an unknown or missing session.
pub fn for_session(app: &App, session_id: Option<&str>) -> Option<String> {
    session_id.and_then(|id| app.sessions.get(id)).map(|s| line(&s))
}

/// Slack's link syntax (mrkdwn) for the same line.
pub fn slack(line: &str) -> String {
    line.replace(LINK, &format!("<{URL}|Launchpad>"))
}

/// Who triggered a GitHub publication, from Home's own view of the request (never from its text).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// An agent or bot published on its own: its `gh` or API calls, `launchpad pr publish`, When done = Draft PR or
    /// Open PR. Also when the owner asked it to post.
    Automatic,
    /// The owner clicked a publication button or card (Post, Publish, Publish all, the extension's Insert or
    /// prefill that the owner submits with GitHub's own button).
    Manual,
}

impl Origin {
    /// The origin of an API call: an agent's session token (`AgentSession`) publishes automatically, the owner's
    /// window manually.
    pub fn of_caller(agent: bool) -> Self {
        if agent { Origin::Automatic } else { Origin::Manual }
    }
    fn words(self) -> &'static str {
        match self {
            Origin::Automatic => "posted automatically",
            Origin::Manual => "posted manually",
        }
    }
}

const ORIGINS: [Origin; 2] = [Origin::Automatic, Origin::Manual];

/// `line` (from `text` or `line`) saying who triggered the publication, inside its italics:
/// `_Written by X via [Launchpad](…), posted manually_`.
pub fn with_origin(line: &str, origin: Origin) -> String {
    let base = without_origin(line);
    match base.as_str().strip_prefix('_').and_then(|b| b.strip_suffix('_')) {
        Some(inner) => format!("_{inner}, {}_", origin.words()),
        None => format!("{base}, {}", origin.words()),
    }
}

/// The GitHub line of a session this machine knows, for a publication of that origin.
pub fn github_line(app: &App, session_id: Option<&str>, origin: Origin) -> Option<String> {
    for_session(app, session_id).map(|l| with_origin(&l, origin))
}

/// `line` without its origin, italic or plain.
fn without_origin(line: &str) -> String {
    let l = line.trim();
    let (inner, italic) = match l.strip_prefix('_').and_then(|x| x.strip_suffix('_')) {
        Some(i) => (i, true),
        None => (l, false),
    };
    let inner = strip_origin(inner);
    if italic { format!("_{inner}_") } else { inner.to_string() }
}

/// The text of a line without a trailing `, posted …`.
fn strip_origin(inner: &str) -> &str {
    ORIGINS.iter().find_map(|o| inner.strip_suffix(o.words()).and_then(|r| r.strip_suffix(", "))).unwrap_or(inner)
}

/// What comes between the text and the line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Separator {
    /// A blank line: pull request and issue bodies, chat messages.
    Blank,
    /// A blank line, `---` and a blank line: comments and reviews (the blank line before `---` keeps the text above
    /// from becoming a heading).
    Rule,
}

/// `body` ending with `line`, once.
pub fn sign(body: &str, line: &str, sep: Separator) -> String {
    let base = unsigned(body);
    if base.is_empty() {
        return line.to_string();
    }
    match sep {
        Separator::Blank => format!("{base}\n\n{line}"),
        Separator::Rule => format!("{base}\n\n---\n\n{line}"),
    }
}

/// Is this the last line of an attribution (any agent or bot, italic or plain, with or without its origin, or
/// Claude Code's default)?
fn is_attribution(l: &str) -> bool {
    let l = l.trim();
    if l == CLAUDE_DEFAULT {
        return true;
    }
    let inner = l.strip_prefix('_').and_then(|x| x.strip_suffix('_')).unwrap_or(l);
    let inner = strip_origin(inner);
    let connector = if inner.starts_with("Written by ") {
        "via"
    } else if inner.starts_with("Done with ") || inner.starts_with("Done by ") {
        "using"
    } else {
        return false;
    };
    inner.ends_with(&format!(" {connector} {LINK}")) || inner.ends_with(&format!(" {connector} <{URL}|Launchpad>"))
}

/// `body` without a trailing attribution line and the separator before it.
pub fn unsigned(body: &str) -> &str {
    let mut t = body.trim_end();
    loop {
        let (rest, last) = match t.rfind('\n') {
            Some(i) => (&t[..i], &t[i + 1..]),
            None => ("", t),
        };
        if !is_attribution(last) {
            return t;
        }
        let rest = rest.trim_end();
        t = rest.strip_suffix("---").filter(|r| r.is_empty() || r.ends_with('\n')).map(str::trim_end).unwrap_or(rest);
    }
}

/// GraphQL mutations that write a description (`input.body`), then those that write a comment or review.
const GRAPHQL_DESCRIPTIONS: &[&str] = &["createPullRequest", "updatePullRequest", "createIssue", "updateIssue"];
const GRAPHQL_COMMENTS: &[&str] = &["addComment", "updateIssueComment", "addPullRequestReview", "submitPullRequestReview", "updatePullRequestReview", "addDiscussionComment", "updateDiscussionComment", "createDiscussion"];

/// Does the request write a body the proxy signs? (It then reads the request body.)
pub fn signs_github(host: &str, method: &str, path: &str) -> bool {
    host.eq_ignore_ascii_case("api.github.com") && (rest_place(method, path).is_some() || (method == "POST" && path.split('?').next() == Some("/graphql")))
}

/// REST endpoints whose JSON `body` is a description or a comment. Inline review comments are left alone: the
/// review's own body carries the line.
fn rest_place(method: &str, path: &str) -> Option<Separator> {
    let segs: Vec<&str> = path.split('?').next().unwrap_or("").trim_matches('/').split('/').collect();
    match (method, segs.as_slice()) {
        ("POST", ["repos", _, _, "pulls" | "issues"]) | ("PATCH", ["repos", _, _, "pulls" | "issues", _]) => Some(Separator::Blank),
        ("POST", ["repos", _, _, "issues", _, "comments"])
        | ("PATCH", ["repos", _, _, "issues", "comments", _])
        | ("POST", ["repos", _, _, "pulls", _, "reviews"])
        | ("PUT", ["repos", _, _, "pulls", _, "reviews", _])
        | ("POST", ["repos", _, _, "pulls", _, "reviews", _, "events"]) => Some(Separator::Rule),
        _ => None,
    }
}

/// Whether a GraphQL document calls `name(…)` (a whole field name).
fn calls(query: &str, name: &str) -> bool {
    query.match_indices(name).any(|(i, _)| {
        let before = query[..i].chars().next_back();
        let after = query[i + name.len()..].trim_start().chars().next();
        !before.is_some_and(|c| c.is_alphanumeric() || c == '_') && after == Some('(')
    })
}

/// A GitHub API request body with the line added, or None when the request is left unchanged (another endpoint,
/// no text, not JSON, an approval without a comment).
pub fn sign_github(method: &str, path: &str, body: &[u8], line: &str) -> Option<Vec<u8>> {
    let mut v: Value = serde_json::from_slice(body).ok()?;
    let p = path.split('?').next().unwrap_or("");
    let (sep, field) = if method == "POST" && p == "/graphql" {
        let query = v["query"].as_str()?.trim_start();
        if !query.starts_with("mutation") {
            return None;
        }
        let sep = if GRAPHQL_DESCRIPTIONS.iter().any(|n| calls(query, n)) {
            Separator::Blank
        } else if GRAPHQL_COMMENTS.iter().any(|n| calls(query, n)) {
            Separator::Rule
        } else {
            return None;
        };
        (sep, v.get_mut("variables")?.get_mut("input")?.get_mut("body")?)
    } else {
        (rest_place(method, p)?, v.get_mut("body")?)
    };
    let text = field.as_str()?;
    // An empty review or comment (an approval alone) stays empty.
    if sep == Separator::Rule && text.trim().is_empty() {
        return None;
    }
    let signed = sign(text, line, sep);
    if signed == text {
        return None;
    }
    *field = Value::String(signed);
    serde_json::to_vec(&v).ok()
}

/// The proxy's part: the signed body of a session's GitHub request, when it writes one. The agent's own request
/// is always an automatic publication, whatever its text says.
pub fn sign_request(app: &App, session_id: &str, host: &str, method: &str, path: &str, body: &[u8]) -> Option<Vec<u8>> {
    if !signs_github(host, method, path) {
        return None;
    }
    let line = github_line(app, Some(session_id), Origin::Automatic)?;
    sign_github(method, path, body, &line)
}

/// The sentence of the contained instructions: what Launchpad signs, and what the agent signs itself.
pub fn instruction(line: &str) -> String {
    let line = without_origin(line);
    let auto = with_origin(&line, Origin::Automatic);
    format!(
        "Attribution: Launchpad ends what you publish through it with the line `{line}`: pull request and issue bodies, comments and reviews you post with `gh` or the GitHub API, Local PRs, Suggestions and Outbox messages. On GitHub, Launchpad adds who triggered the publication inside the line: `, posted automatically` when you or another agent publish (also when the owner asked you to), `, posted manually` when the owner clicks Post or Publish. Do not add that line, its ending or another attribution line (such as \"Generated with …\") to them yourself. Bot channel and DM posts (`launchpad channel post`, `launchpad dm`), your replies and everything else you write inside Launchpad itself (proposals, reports, the knowledge base, bot memory) never get an attribution line. When you publish text outside Launchpad yourself, end it with `{auto}` after a blank line.\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLAUDE: &str = "_Written by Claude Code via [Launchpad](https://github.com/Wauplin/launchpad)_";

    #[test]
    fn instruction_keeps_channel_posts_unsigned() {
        let t = instruction(CLAUDE);
        assert!(t.contains("`launchpad channel post`, `launchpad dm`), your replies and everything else you write inside Launchpad itself"), "{t}");
        assert!(t.contains("When you publish text outside Launchpad yourself, end it with `_Written by Claude Code via [Launchpad](https://github.com/Wauplin/launchpad), posted automatically_`"), "{t}");
        assert!(t.contains("with the line `_Written by Claude Code via [Launchpad](https://github.com/Wauplin/launchpad)_`"), "{t}");
        assert!(t.contains("Do not add that line, its ending"), "{t}");
        // Given a line with an origin, the sentence still names the base line.
        assert_eq!(instruction(&with_origin(CLAUDE, Origin::Manual)), t);
    }

    const AUTO: &str = "_Written by Claude Code via [Launchpad](https://github.com/Wauplin/launchpad), posted automatically_";
    const MANUAL: &str = "_Written by Claude Code via [Launchpad](https://github.com/Wauplin/launchpad), posted manually_";

    #[test]
    fn origin_goes_inside_the_italics() {
        assert_eq!(with_origin(CLAUDE, Origin::Automatic), AUTO);
        assert_eq!(with_origin(CLAUDE, Origin::Manual), MANUAL);
        // A line that already says one origin gets the other, never both.
        assert_eq!(with_origin(AUTO, Origin::Manual), MANUAL);
        assert_eq!(with_origin(MANUAL, Origin::Manual), MANUAL);
        let bot = with_origin(&text(Some("Hubert.py Bot"), "Codex"), Origin::Automatic);
        assert_eq!(bot, "_Written by Hubert.py Bot via [Launchpad](https://github.com/Wauplin/launchpad), posted automatically_");
        assert_eq!(with_origin("Written by Codex via [Launchpad](https://github.com/Wauplin/launchpad)", Origin::Manual), "Written by Codex via [Launchpad](https://github.com/Wauplin/launchpad), posted manually");
        // The owner's window publishes manually, an agent's session token automatically.
        assert_eq!(Origin::of_caller(false), Origin::Manual);
        assert_eq!(Origin::of_caller(true), Origin::Automatic);
    }

    #[test]
    fn footers_with_an_origin_are_replaced_not_doubled() {
        for (line, other) in [(AUTO, MANUAL), (MANUAL, AUTO)] {
            let once = sign("Fixes it.", line, Separator::Rule);
            assert_eq!(once, format!("Fixes it.\n\n---\n\n{line}"));
            assert_eq!(sign(&once, line, Separator::Rule), once);
            // A stale footer of the other origin, or one without an origin, is replaced by the trusted one.
            assert_eq!(sign(&sign("Fixes it.", other, Separator::Rule), line, Separator::Rule), once);
            assert_eq!(sign(&sign("Fixes it.", CLAUDE, Separator::Rule), line, Separator::Rule), once);
            assert_eq!(sign(&format!("Fixes it.\n\n{other}\n\n---\n\n{CLAUDE}"), line, Separator::Rule), once);
            assert_eq!(unsigned(&once), "Fixes it.");
            assert_eq!(unsigned(&format!("Fixes it.\n\n{}", other.trim_matches('_'))), "Fixes it.");
        }
        // Text that only mentions the words is not a footer.
        assert_eq!(unsigned("This was posted manually"), "This was posted manually");
    }

    #[test]
    fn an_agents_own_request_cannot_claim_a_manual_post() {
        // The agent writes a manual footer itself; the proxy's line (automatic) replaces it.
        let forged = serde_json::to_vec(&serde_json::json!({"body": format!("Hi\n\n---\n\n{MANUAL}")})).unwrap();
        let c = sign_github("POST", "/repos/o/r/issues/3/comments", &forged, AUTO).unwrap();
        assert_eq!(body(&c)["body"], format!("Hi\n\n---\n\n{AUTO}"));
        let pr = serde_json::to_vec(&serde_json::json!({"title": "t", "body": format!("Desc\n\n{MANUAL}")})).unwrap();
        assert_eq!(body(&sign_github("POST", "/repos/o/r/pulls", &pr, AUTO).unwrap())["body"], format!("Desc\n\n{AUTO}"));
        assert!(sign_github("POST", "/repos/o/r/issues/3/comments", &c, AUTO).is_none());
    }

    #[test]
    fn wording_names_the_bot_or_the_harness() {
        assert_eq!(text(None, "Claude Code"), CLAUDE);
        assert_eq!(text(None, "Codex"), "_Written by Codex via [Launchpad](https://github.com/Wauplin/launchpad)_");
        assert_eq!(text(None, "OpenCode"), "_Written by OpenCode via [Launchpad](https://github.com/Wauplin/launchpad)_");
        assert_eq!(text(Some("HFJS Bot"), "Claude Code"), "_Written by HFJS Bot via [Launchpad](https://github.com/Wauplin/launchpad)_");
        assert_eq!(text(Some("HFJS Bot"), "Codex"), "_Written by HFJS Bot via [Launchpad](https://github.com/Wauplin/launchpad)_");
        assert_eq!(text(Some(" "), "Codex"), "_Written by Codex via [Launchpad](https://github.com/Wauplin/launchpad)_");
        assert_eq!(slack(CLAUDE), "_Written by Claude Code via <https://github.com/Wauplin/launchpad|Launchpad>_");
    }

    #[test]
    fn signing_is_idempotent_and_replaces_other_lines() {
        let once = sign("Fixes it.", CLAUDE, Separator::Rule);
        assert_eq!(once, format!("Fixes it.\n\n---\n\n{CLAUDE}"));
        assert_eq!(sign(&once, CLAUDE, Separator::Rule), once);
        assert_eq!(sign(&format!("{once}\n"), CLAUDE, Separator::Rule), once);
        let pr = sign("Body", CLAUDE, Separator::Blank);
        assert_eq!(pr, format!("Body\n\n{CLAUDE}"));
        assert_eq!(sign(&pr, CLAUDE, Separator::Blank), pr);
        // Claude Code's default line, a plain line or another session's line is replaced.
        assert_eq!(sign("Body\n\n🤖 Generated with [Claude Code](https://claude.com/claude-code)", CLAUDE, Separator::Blank), pr);
        assert_eq!(sign("Body\n\nWritten by Codex via [Launchpad](https://github.com/Wauplin/launchpad)", CLAUDE, Separator::Blank), pr);
        // Old Launchpad footers on saved drafts are replaced too, in Markdown and Slack.
        for name in ["Done with Claude Code", "Done by Hubert.js Bot"] {
            for link in [LINK.to_string(), format!("<{URL}|Launchpad>")] {
                for footer in [format!("{name} using {link}"), format!("_{name} using {link}_")] {
                    assert_eq!(sign(&format!("Body\n\n---\n\n{footer}"), CLAUDE, Separator::Blank), pr);
                }
            }
        }
        let bot = text(Some("HFJS Bot"), "Claude Code");
        assert_eq!(sign(&once, &bot, Separator::Rule), format!("Fixes it.\n\n---\n\n{bot}"));
        // A rule that is not before the line stays; an empty body gets the line alone.
        assert_eq!(sign("a\n---\nb", CLAUDE, Separator::Blank), format!("a\n---\nb\n\n{CLAUDE}"));
        assert_eq!(sign("", CLAUDE, Separator::Blank), CLAUDE);
    }

    fn body(v: &[u8]) -> Value {
        serde_json::from_slice(v).unwrap()
    }

    #[test]
    fn github_requests_are_signed_by_kind() {
        let pr = sign_github("POST", "/repos/o/r/pulls", br#"{"title":"t","body":"Text","head":"b"}"#, CLAUDE).unwrap();
        assert_eq!(body(&pr)["body"], format!("Text\n\n{CLAUDE}"));
        assert_eq!(body(&pr)["head"], "b");
        let c = sign_github("POST", "/repos/o/r/issues/3/comments", br#"{"body":"Hi"}"#, CLAUDE).unwrap();
        assert_eq!(body(&c)["body"], format!("Hi\n\n---\n\n{CLAUDE}"));
        // Already signed, an approval without text, inline comments, other endpoints and non-JSON stay unchanged.
        assert!(sign_github("POST", "/repos/o/r/issues/3/comments", &c, CLAUDE).is_none());
        assert!(sign_github("POST", "/repos/o/r/pulls/3/reviews", br#"{"event":"APPROVE","body":""}"#, CLAUDE).is_none());
        assert!(sign_github("POST", "/repos/o/r/pulls/3/comments", br#"{"body":"x"}"#, CLAUDE).is_none());
        assert!(sign_github("PATCH", "/repos/o/r/pulls/3", br#"{"state":"closed"}"#, CLAUDE).is_none());
        assert!(sign_github("POST", "/repos/o/r/issues/3/comments", b"body=x", CLAUDE).is_none());
        assert!(!signs_github("api.github.com", "GET", "/repos/o/r/pulls"));
        assert!(!signs_github("example.com", "POST", "/repos/o/r/pulls"));
        assert!(signs_github("api.github.com", "POST", "/graphql"));
    }

    #[test]
    fn graphql_mutations_of_gh_are_signed() {
        let q = |query: &str, body: &str| serde_json::to_vec(&serde_json::json!({"query": query, "variables": {"input": {"body": body, "subjectId": "X"}}})).unwrap();
        let comment = sign_github("POST", "/graphql", &q("mutation CommentCreate($input: AddCommentInput!) { addComment(input: $input) { commentEdge { node { url } } } }", "Hi"), CLAUDE).unwrap();
        assert_eq!(body(&comment)["variables"]["input"]["body"], format!("Hi\n\n---\n\n{CLAUDE}"));
        let pr = sign_github("POST", "/graphql", &q("mutation PullRequestCreate($input: CreatePullRequestInput!) { createPullRequest(input: $input) { pullRequest { url } } }", "Desc"), CLAUDE).unwrap();
        assert_eq!(body(&pr)["variables"]["input"]["body"], format!("Desc\n\n{CLAUDE}"));
        // Queries, inline review comments and mutations without a body are left alone.
        assert!(sign_github("POST", "/graphql", &q("query { addComment(x: 1) }", "Hi"), CLAUDE).is_none());
        assert!(sign_github("POST", "/graphql", &q("mutation($input: X!) { addPullRequestReviewComment(input: $input) { x } }", "Hi"), CLAUDE).is_none());
        assert!(sign_github("POST", "/graphql", br#"{"query":"mutation($input: X!) { addComment(input: $input) { x } }","variables":{"input":{"subjectId":"X"}}}"#, CLAUDE).is_none());
    }
}

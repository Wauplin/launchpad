//! The GitHub writes the owner's clicks make about an issue or pull request (a suggestion's Post and its follow-ups,
//! Approve and run workflows), and the few live reads they check first. Nothing read here is stored.
use crate::suggestions::{CloseReason, ItemAction};
use anyhow::{Context, Result, ensure};
use launchpad_core::App;
use reqwest::Method;
use serde_json::{Value, json};

pub fn validate(repo: &str) -> Result<()> {
    ensure!(launchpad_core::github::valid_repo(repo), "invalid repository");
    Ok(())
}

/// The pull request `n`, read from GitHub now (REST: `state`, `head.sha`, `head.repo`, `maintainer_can_modify`…).
pub async fn pull(app: &App, repo: &str, n: u64) -> Result<Value> {
    validate(repo)?;
    app.github.get(&format!("/repos/{repo}/pulls/{n}")).await
}

/// The issue or pull request `n`, read from GitHub now (REST: `title`, `pull_request` for a pull request…).
pub async fn issue(app: &App, repo: &str, n: u64) -> Result<Value> {
    validate(repo)?;
    app.github.get(&format!("/repos/{repo}/issues/{n}")).await
}

/// Closes issue or pull request `n` (without merging), with an issue's `state_reason` (completed, not_planned or
/// duplicate) when given. Only the owner's Post of a suggestion calls it.
pub async fn close(app: &App, repo: &str, n: u64, reason: Option<&str>) -> Result<Value> {
    validate(repo)?;
    let mut body = json!({"state": "closed"});
    if let Some(r) = reason {
        body["state_reason"] = json!(r);
    }
    app.github.request(Method::PATCH, &format!("/repos/{repo}/issues/{n}"), Some(body)).await
}

/// Pure: the REST call of an action on `repo#n` (method, path, body); None for the draft state, which only GraphQL
/// changes (`draft_mutation`). `reason` is the close's `state_reason` sent.
pub fn rest_call(repo: &str, n: u64, action: &ItemAction, reason: Option<&str>) -> Option<(Method, String, Value)> {
    let issue = format!("/repos/{repo}/issues/{n}");
    Some(match action {
        ItemAction::Close { .. } => {
            let mut body = json!({"state": "closed"});
            if let Some(r) = reason {
                body["state_reason"] = json!(r);
            }
            (Method::PATCH, issue, body)
        }
        ItemAction::Reopen => (Method::PATCH, issue, json!({"state": "open"})),
        ItemAction::RequestReviewers { reviewers, team_reviewers } => (Method::POST, format!("/repos/{repo}/pulls/{n}/requested_reviewers"), json!({"reviewers": reviewers, "team_reviewers": team_reviewers})),
        ItemAction::UpdateBranch => (Method::PUT, format!("/repos/{repo}/pulls/{n}/update-branch"), json!({})),
        ItemAction::ReadyForReview | ItemAction::ConvertToDraft => return None,
    })
}

/// Pure: the GraphQL mutation that changes a pull request's draft state.
pub fn draft_mutation(action: &ItemAction) -> Option<&'static str> {
    match action {
        ItemAction::ReadyForReview => Some("markPullRequestReadyForReview"),
        ItemAction::ConvertToDraft => Some("convertPullRequestToDraft"),
        _ => None,
    }
}

/// Pure: the `state_reason` of a close.
pub fn close_reason(action: &ItemAction) -> Option<&'static str> {
    match action {
        ItemAction::Close { reason: Some(r), .. } => Some(match r {
            CloseReason::Completed => "completed",
            CloseReason::NotPlanned => "not_planned",
            CloseReason::Duplicate => "duplicate",
        }),
        _ => None,
    }
}

/// Runs one action of a suggestion on `repo#n` (the owner's Post). Each is one GitHub write, never retried.
pub async fn apply(app: &App, repo: &str, n: u64, action: &ItemAction) -> Result<()> {
    validate(repo)?;
    if let Some(mutation) = draft_mutation(action) {
        let pr = pull(app, repo, n).await?;
        let node = pr["node_id"].as_str().context("pull request without node ID")?;
        let v = app.github.request(Method::POST, "/graphql", Some(json!({"query": format!("mutation($id: ID!) {{ {mutation}(input: {{pullRequestId: $id}}) {{ pullRequest {{ isDraft }} }} }}"), "variables": {"id": node}}))).await?;
        ensure!(v["errors"].as_array().is_none_or(|e| e.is_empty()), "{}", v["errors"][0]["message"].as_str().unwrap_or("GitHub refused it"));
        return Ok(());
    }
    let reason = close_reason(action);
    let (method, path, body) = rest_call(repo, n, action, reason).context("no REST call")?;
    match app.github.request(method, &path, Some(body)).await {
        // GitHub refused the duplicate reason (nothing changed): close as not planned; the comment's `Duplicate of #N`
        // line still links the original.
        Err(e) if reason == Some("duplicate") && status_of(&e) == Some(reqwest::StatusCode::UNPROCESSABLE_ENTITY) => close(app, repo, n, Some("not_planned")).await.map(|_| ()),
        r => r.map(|_| ()),
    }
}

fn status_of(e: &anyhow::Error) -> Option<reqwest::StatusCode> {
    e.chain().find_map(|c| c.downcast_ref::<reqwest::Error>()).and_then(|e| e.status())
}

/// Posts `body` as a conversation comment.
pub async fn comment(app: &App, repo: &str, n: u64, body: &str) -> Result<Value> {
    validate(repo)?;
    ensure!(!body.trim().is_empty(), "empty comment");
    app.github.request(Method::POST, &format!("/repos/{repo}/issues/{n}/comments"), Some(json!({"body": body}))).await
}

/// Posts a review: `event` (COMMENT, APPROVE or REQUEST_CHANGES), `body`, the reviewed `commit_id` and optional line
/// comments (path, line, side LEFT or RIGHT, body, and an optional start_line on the same side for a range).
pub async fn review(app: &App, repo: &str, n: u64, body: &Value) -> Result<Value> {
    validate(repo)?;
    ensure!(["COMMENT", "APPROVE", "REQUEST_CHANGES"].contains(&body["event"].as_str().unwrap_or("")), "invalid review");
    ensure!(body["commit_id"].as_str().is_some(), "missing reviewed commit");
    let mut review = json!({"event": body["event"], "body": body["body"], "commit_id": body["commit_id"]});
    if let Some(comments) = body["comments"].as_array().filter(|c| !c.is_empty()) {
        ensure!(comments.len() <= 100, "too many line comments");
        let mut out = Vec::with_capacity(comments.len());
        for c in comments {
            let (Some(path), Some(line), Some(text)) = (c["path"].as_str(), c["line"].as_u64(), c["body"].as_str()) else { anyhow::bail!("invalid line comment") };
            let side = c["side"].as_str().unwrap_or("RIGHT");
            ensure!(side == "LEFT" || side == "RIGHT", "invalid line comment side");
            ensure!(!text.trim().is_empty(), "empty line comment");
            let mut comment = json!({"path": path, "line": line, "side": side, "body": text});
            if let Some(start) = c["start_line"].as_u64() {
                ensure!(start > 0 && start < line, "invalid line comment range");
                comment["start_line"] = json!(start);
                comment["start_side"] = json!(side);
            }
            out.push(comment);
        }
        review["comments"] = Value::Array(out);
    }
    app.github.request(Method::POST, &format!("/repos/{repo}/pulls/{n}/reviews"), Some(review)).await
}

/// Merges pull request `n` at `sha` (the reviewed head) with `method` (merge, squash or rebase).
pub async fn merge(app: &App, repo: &str, n: u64, sha: &str, method: &str) -> Result<Value> {
    validate(repo)?;
    ensure!(["merge", "squash", "rebase"].contains(&method), "invalid merge method");
    ensure!(!sha.is_empty(), "missing reviewed head");
    let v = app.github.request(Method::PUT, &format!("/repos/{repo}/pulls/{n}/merge"), Some(json!({"sha": sha, "merge_method": method}))).await?;
    ensure!(v["merged"] == true, "{}", v["message"]);
    Ok(v)
}

/// Posts `body` as a reply in the review thread of review comment `comment` of pull request `n` (GitHub's
/// review-comment reply API). Only the owner's Post of a suggestion calls it.
pub async fn reply(app: &App, repo: &str, n: u64, comment: u64, body: &str) -> Result<Value> {
    validate(repo)?;
    ensure!(!body.trim().is_empty(), "empty comment");
    app.github.request(Method::POST, &format!("/repos/{repo}/pulls/{n}/comments/{comment}/replies"), Some(json!({"body": body}))).await
}

/// Approves the GitHub Actions runs of the PR's head that wait for a maintainer (pull requests from forks of
/// first-time contributors), so they start. `head` is the commit the owner or the agent inspected: a newer head
/// refuses, since approving runs executes the PR's code in the repository's CI.
pub async fn approve_runs(app: &App, repo: &str, n: u64, head: &str) -> Result<Value> {
    ensure!(head.len() == 40 && head.bytes().all(|b| b.is_ascii_hexdigit()), "give the inspected head commit (40 characters)");
    let head = head.to_ascii_lowercase();
    let current = pull(app, repo, n).await?;
    ensure!(current["state"] == "open", "pull request is no longer open");
    ensure!(current["head"]["sha"].as_str() == Some(head.as_str()), "PR head changed; inspect the new commits before running CI");
    let runs = app.github.get(&format!("/repos/{repo}/actions/runs?head_sha={head}&status=action_required&per_page=100")).await?;
    let ids: Vec<u64> = runs["workflow_runs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["head_sha"].as_str() == Some(head.as_str()) && (r["conclusion"] == "action_required" || r["status"] == "action_required"))
        .filter_map(|r| r["id"].as_u64())
        .collect();
    ensure!(!ids.is_empty(), "no workflow run of {} waits for approval", &head[..7]);
    for id in &ids {
        app.github.request(Method::POST, &format!("/repos/{repo}/actions/runs/{id}/approve"), None).await?;
    }
    Ok(json!({"approved": ids.len()}))
}

/// Pure: the changed files of a pull request (GitHub's `pulls/{n}/files` entries: `filename`, and `previous_filename`
/// for a rename) that are CI configuration: GitHub Actions workflows and local actions (`.github/workflows/`,
/// `.github/actions/`, any `action.yml`), and the configuration of other CI services. An agent never approves the
/// runs of a pull request that touches one: its workflows would run as the pull request wrote them.
pub fn ci_config_changes(files: &[Value]) -> Vec<String> {
    let mut out: Vec<String> = files
        .iter()
        .flat_map(|f| [f["filename"].as_str(), f["previous_filename"].as_str()])
        .flatten()
        .filter(|p| is_ci_config(p))
        .map(str::to_string)
        .collect();
    out.dedup();
    out
}
fn is_ci_config(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    let name = p.rsplit('/').next().unwrap_or(&p);
    p.starts_with(".github/workflows/")
        || p.starts_with(".github/actions/")
        || matches!(name, "action.yml" | "action.yaml")
        || p.starts_with(".circleci/")
        || p.starts_with(".buildkite/")
        || p.starts_with(".woodpecker/")
        || matches!(p.as_str(), ".gitlab-ci.yml" | ".travis.yml" | "azure-pipelines.yml" | "jenkinsfile" | "appveyor.yml" | ".appveyor.yml" | "bitbucket-pipelines.yml" | ".drone.yml" | ".woodpecker.yml" | "cloudbuild.yaml" | "cloudbuild.yml")
}

/// The most files a CI approval checks: GitHub lists at most 3,000 changed files of a pull request.
const CHECKED_FILES: usize = 3000;

/// An agent's CI approval (`launchpad ci approve`): approves the GitHub Actions runs of pull request `n` that wait
/// for a maintainer, only at `head` (the commit the agent read), and only when the pull request changes no CI
/// configuration (`ci_config_changes`). Home reads the pull request and its files live, checks the head again after
/// reading them (so the files checked are those of `head`), then approves.
pub async fn agent_approve_runs(app: &App, repo: &str, n: u64, head: &str) -> Result<Value> {
    ensure!(head.len() == 40 && head.bytes().all(|b| b.is_ascii_hexdigit()), "--head takes the 40-character commit you inspected");
    let head = head.to_ascii_lowercase();
    let (now, files) = diff(app, repo, n).await?;
    check_agent_approval(&head, &now, files.as_array().map(Vec::as_slice).unwrap_or_default())?;
    approve_runs(app, repo, n, &head).await
}

/// Pure: whether an agent may approve the runs of `asked` when the pull request's head is `now` with these changed
/// files: the same head, and no CI configuration change among files GitHub listed in full.
pub fn check_agent_approval(asked: &str, now: &str, files: &[Value]) -> Result<()> {
    ensure!(now.eq_ignore_ascii_case(asked), "the pull request's head moved to {}: read the new commits, then approve that head", &now[..now.len().min(12)]);
    ensure!(files.len() < CHECKED_FILES, "the pull request changes too many files to check: the owner approves its workflows");
    let ci = ci_config_changes(files);
    ensure!(ci.is_empty(), "refused: the pull request changes CI configuration ({}); only the owner approves its workflows", ci.join(", "));
    Ok(())
}

/// The head commit of pull request `n` and its changed files (each with its `patch`), read from GitHub now.
pub async fn diff(app: &App, repo: &str, n: u64) -> Result<(String, Value)> {
    let current = pull(app, repo, n).await?;
    let mut files = vec![];
    for page in 1..=30 {
        let batch = app.github.get(&format!("/repos/{repo}/pulls/{n}/files?per_page=100&page={page}")).await?;
        let batch = batch.as_array().cloned().unwrap_or_default();
        let count = batch.len();
        files.extend(batch);
        if count < 100 {
            break;
        }
    }
    Ok((current["head"]["sha"].as_str().unwrap_or_default().to_string(), Value::Array(files)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_action_is_one_github_call() {
        let call = |a: &ItemAction| rest_call("o/r", 7, a, close_reason(a)).map(|(m, p, b)| (m.to_string(), p, b));
        let close = ItemAction::Close { reason: None, duplicate_of: None };
        assert_eq!(call(&close), Some(("PATCH".into(), "/repos/o/r/issues/7".into(), json!({"state": "closed"}))));
        let not_planned = ItemAction::Close { reason: Some(CloseReason::NotPlanned), duplicate_of: None };
        assert_eq!(call(&not_planned).unwrap().2, json!({"state": "closed", "state_reason": "not_planned"}));
        let dup = ItemAction::Close { reason: Some(CloseReason::Duplicate), duplicate_of: Some(3) };
        assert_eq!(call(&dup).unwrap().2, json!({"state": "closed", "state_reason": "duplicate"}));
        assert_eq!(call(&ItemAction::Reopen), Some(("PATCH".into(), "/repos/o/r/issues/7".into(), json!({"state": "open"}))));
        let review = ItemAction::RequestReviewers { reviewers: vec!["ana".into()], team_reviewers: vec!["core".into()] };
        assert_eq!(call(&review), Some(("POST".into(), "/repos/o/r/pulls/7/requested_reviewers".into(), json!({"reviewers": ["ana"], "team_reviewers": ["core"]}))));
        assert_eq!(call(&ItemAction::UpdateBranch).unwrap().1, "/repos/o/r/pulls/7/update-branch");
        // The draft state only changes through GraphQL.
        assert_eq!(call(&ItemAction::ReadyForReview), None);
        assert_eq!(draft_mutation(&ItemAction::ReadyForReview), Some("markPullRequestReadyForReview"));
        assert_eq!(draft_mutation(&ItemAction::ConvertToDraft), Some("convertPullRequestToDraft"));
        assert_eq!(draft_mutation(&close), None);
    }

    #[test]
    fn ci_config_changes_are_refused() {
        let files = |names: &[(&str, Option<&str>)]| names.iter().map(|(f, prev)| json!({"filename": f, "previous_filename": prev})).collect::<Vec<_>>();
        assert!(ci_config_changes(&files(&[("src/lib.rs", None), ("README.md", None), ("SECURITY.md", None), ("docs/github/workflows.md", None)])).is_empty());
        for bad in [".github/workflows/ci.yml", ".github/actions/setup/run.sh", "tools/action.yml", "sub/dir/Action.YAML", ".gitlab-ci.yml", ".circleci/config.yml", "azure-pipelines.yml", "Jenkinsfile"] {
            assert_eq!(ci_config_changes(&files(&[("src/a.rs", None), (bad, None)])), vec![bad.to_string()], "{bad}");
        }
        // A workflow renamed away counts too.
        assert_eq!(ci_config_changes(&files(&[("old.yml", Some(".github/workflows/release.yml"))])), vec![".github/workflows/release.yml".to_string()]);
    }

    #[test]
    fn agents_approve_only_the_head_they_read_without_ci_changes() {
        let (a, b) = ("a".repeat(40), "b".repeat(40));
        let ok = vec![json!({"filename": "src/lib.rs"})];
        assert!(check_agent_approval(&a, &a, &ok).is_ok());
        assert!(check_agent_approval(&a, &a.to_uppercase(), &ok).is_ok());
        let moved = check_agent_approval(&a, &b, &ok).unwrap_err().to_string();
        assert!(moved.contains("moved to bbbbbbbbbbbb"), "{moved}");
        let wf = vec![json!({"filename": "src/lib.rs"}), json!({"filename": ".github/workflows/ci.yml"})];
        let refused = check_agent_approval(&a, &a, &wf).unwrap_err().to_string();
        assert!(refused.contains(".github/workflows/ci.yml"), "{refused}");
        let many: Vec<Value> = (0..CHECKED_FILES).map(|i| json!({"filename": format!("f{i}")})).collect();
        assert!(check_agent_approval(&a, &a, &many).is_err(), "a list GitHub may have cut is refused");
    }
}

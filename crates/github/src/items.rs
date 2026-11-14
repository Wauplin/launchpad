//! The GitHub writes the owner's clicks make about an issue or pull request (a suggestion's Post and its follow-ups,
//! Approve and run workflows), and the few live reads they check first. Nothing read here is stored.
use anyhow::{Result, ensure};
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

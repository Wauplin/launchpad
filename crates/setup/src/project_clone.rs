//! Clone from GitHub: Home mirrors a GitHub repository and registers it as a Git folder (Folders → Add folder).
use std::collections::HashSet;
use std::time::Duration;

use anyhow::{Result, bail};
use serde::Serialize;
use serde_json::Value;
use ts_rs::TS;

use launchpad_core::{App, model::Project};

/// `owner/repo` from `owner/repo`, `https://github.com/owner/repo(.git)` or `git@github.com:owner/repo.git`.
pub fn parse_repo(input: &str) -> Result<(String, String)> {
    let s = input.trim().trim_end_matches('/');
    let s = s
        .strip_prefix("https://github.com/")
        .or_else(|| s.strip_prefix("http://github.com/"))
        .or_else(|| s.strip_prefix("github.com/"))
        .or_else(|| s.strip_prefix("git@github.com:"))
        .unwrap_or(s);
    let s = s.strip_suffix(".git").unwrap_or(s);
    let valid = |part: &str| !part.is_empty() && !part.starts_with(['.', '-']) && part.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    match s.split_once('/') {
        Some((owner, repo)) if valid(owner) && valid(repo) => Ok((owner.into(), repo.into())),
        _ => bail!("enter a GitHub repository as owner/name"),
    }
}

/// A repository offered by Clone from GitHub: one the owner pushed to recently.
#[derive(Clone, Debug, Serialize, TS)]
pub struct CloneSuggestion {
    pub repo: String,
}

/// The owner's recently pushed repositories without the registered ones, read from GitHub now, best effort (GitHub
/// may be signed out or unreachable: no suggestions then).
pub async fn suggestions(app: &App) -> Result<Vec<CloneSuggestion>> {
    let registered: HashSet<String> = launchpad_core::projects::list(app).await?.into_iter().filter_map(|p| p.gh_repo).map(|r| r.to_lowercase()).collect();
    let read = tokio::time::timeout(Duration::from_secs(15), app.github.get("/user/repos?sort=pushed&per_page=30")).await.unwrap_or_else(|_| Err(anyhow::anyhow!("timed out")));
    let pushed = match read {
        Ok(v) => v.as_array().cloned().unwrap_or_default(),
        Err(e) => {
            tracing::debug!("clone suggestions: {e:#}");
            vec![]
        }
    };
    Ok(pick(&pushed, &registered))
}

/// The repositories of GitHub's list that are not archived, valid and not registered yet, in its order.
fn pick(pushed: &[Value], registered: &HashSet<String>) -> Vec<CloneSuggestion> {
    let mut taken = HashSet::new();
    pushed
        .iter()
        .filter(|r| r["archived"].as_bool() != Some(true))
        .filter_map(|r| r["full_name"].as_str())
        .filter(|r| parse_repo(r).is_ok() && !registered.contains(&r.to_lowercase()) && taken.insert(r.to_lowercase()))
        .map(|repo| CloneSuggestion { repo: repo.to_string() })
        .collect()
}

/// A GitHub repository as a Git folder of Home: Home mirrors it; runners clone it on first use. A repository
/// already in another workspace becomes a second folder.
pub async fn clone_to_home(app: &App, input: &str, workspace_id: &str) -> Result<Project> {
    let (owner, repo) = parse_repo(input)?;
    launchpad_core::projects::register_remote(app, &repo, &format!("https://github.com/{owner}/{repo}.git"), workspace_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_pushed_repositories_not_registered_yet() {
        let pushed = serde_json::json!([{"full_name": "a/one"}, {"full_name": "d/mine"}, {"full_name": "b/old", "archived": true}, {"full_name": "A/One"}, {"full_name": "bad"}]);
        let registered = HashSet::from(["d/mine".to_string()]);
        let out = pick(pushed.as_array().unwrap(), &registered);
        assert_eq!(out.iter().map(|s| s.repo.as_str()).collect::<Vec<_>>(), ["a/one"]);
    }
}

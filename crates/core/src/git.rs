//! Git helpers. Shells out to the `git` CLI (it uses your SSH configuration as is), except for reading
//! remotes, which parses `.git/config` directly so that scanning 200 folders stays fast.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::shellenv::UserEnv;

/// The git directory of a checkout: `.git/`, or the target of a `.git` file (worktrees, submodules).
pub fn git_dir(repo: &Path) -> Option<PathBuf> {
    let dot = repo.join(".git");
    if dot.is_dir() {
        return Some(dot);
    }
    let text = std::fs::read_to_string(&dot).ok()?;
    let target = text.strip_prefix("gitdir:")?.trim();
    let p = Path::new(target);
    Some(if p.is_absolute() { p.to_path_buf() } else { repo.join(p) })
}

/// Remote URLs from the git config: `[(name, url)]`.
pub fn remotes(repo: &Path) -> Vec<(String, String)> {
    let Some(dir) = git_dir(repo) else { return vec![] };
    // Worktrees keep their config in the main repository (`commondir`).
    let common = std::fs::read_to_string(dir.join("commondir"))
        .ok()
        .map(|c| {
            let p = Path::new(c.trim());
            if p.is_absolute() { p.to_path_buf() } else { dir.join(p) }
        })
        .unwrap_or(dir);
    let Ok(text) = std::fs::read_to_string(common.join("config")) else { return vec![] };
    let mut out = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            current = line
                .strip_prefix("[remote \"")
                .and_then(|r| r.strip_suffix("\"]"))
                .map(str::to_string);
        } else if let (Some(name), Some((k, v))) = (&current, line.split_once('='))
            && k.trim() == "url" {
            out.push((name.clone(), v.trim().to_string()));
        }
    }
    out
}

/// `owner/repo` from a GitHub remote URL (SSH or HTTPS form).
pub fn github_repo(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("git@github.com:")
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))
        .or_else(|| url.strip_prefix("https://github.com/"))
        .or_else(|| url.strip_prefix("http://github.com/"))
        .or_else(|| url.strip_prefix("git://github.com/"))?;
    let rest = rest.trim_end_matches('/').trim_end_matches(".git");
    let mut it = rest.split('/');
    let (owner, repo) = (it.next()?, it.next()?);
    if owner.is_empty() || repo.is_empty() || it.next().is_some() {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

/// Local branches, the current one first.
pub async fn branches(env: &UserEnv, repo: &Path) -> Result<(Option<String>, Vec<String>)> {
    let current = crate::procs::output(env, "git", &["branch", "--show-current"], Some(repo)).await.ok().filter(|b| !b.is_empty());
    let list = crate::procs::output(
        env,
        "git",
        &["for-each-ref", "--sort=-committerdate", "--format=%(refname:short)", "refs/heads"],
        Some(repo),
    )
    .await?;
    let mut names: Vec<String> = list.lines().map(str::to_string).filter(|l| !l.is_empty()).collect();
    if let Some(c) = &current {
        names.retain(|b| b != c);
        names.insert(0, c.clone());
    }
    Ok((current, names))
}

#[cfg(test)]
mod tests {
    use super::github_repo;

    #[test]
    fn parses_github_remotes() {
        assert_eq!(github_repo("git@github.com:huggingface/huggingface_hub.git").as_deref(), Some("huggingface/huggingface_hub"));
        assert_eq!(github_repo("https://github.com/huggingface/huggingface.js").as_deref(), Some("huggingface/huggingface.js"));
        assert_eq!(github_repo("ssh://git@github.com/a/b.git/").as_deref(), Some("a/b"));
        assert_eq!(github_repo("git@gitlab.com:a/b.git"), None);
        assert_eq!(github_repo("https://github.com/a"), None);
    }
}

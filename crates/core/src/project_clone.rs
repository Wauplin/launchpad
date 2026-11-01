//! Clone a GitHub repository into the projects root and register it (Projects → Clone from GitHub).
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::Value;
use ts_rs::TS;

use crate::{App, model::Project};

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

/// A repository offered by Clone from GitHub. `notifications`: GitHub notification threads updated in the last 30 days.
#[derive(Clone, Debug, Serialize, TS)]
pub struct CloneSuggestion {
    pub repo: String,
    pub notifications: u32,
}

/// Cached GitHub responses read by `suggestions` and written by `refresh_suggestions`. They use their own keys so the
/// inbox's conditional requests are never affected.
const NOTIFICATIONS: &str = "/notifications?all=true&per_page=50";
const PUSHED: &str = "/user/repos?sort=pushed&per_page=30";
const RECENT_DAYS: i64 = 30;

/// Repositories from the GitHub cache, without the registered ones: repositories with recent notifications first
/// (most threads, then latest activity), then the owner's recently pushed repositories, then other cached ones by name.
pub async fn suggestions(app: &App) -> Result<Vec<CloneSuggestion>> {
    let registered: HashSet<String> = crate::projects::list(app).await?.into_iter().filter_map(|p| p.gh_repo).map(|r| r.to_lowercase()).collect();
    let (threads, cached, responses) = app
        .cache_db
        .call(|c| {
            let mut stmt = c.prepare("SELECT id, json_extract(json, '$.repo'), updated_at FROM inbox_thread")?;
            let threads = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, String>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            let mut stmt = c.prepare("SELECT repo FROM gh_item WHERE repo IS NOT NULL")?;
            let cached = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            let mut stmt = c.prepare("SELECT url, json FROM gh_response WHERE url IN (?1, ?2)")?;
            let responses = stmt.query_map([NOTIFICATIONS, PUSHED], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?.collect::<rusqlite::Result<HashMap<_, _>>>()?;
            Ok((threads, cached, responses))
        })
        .await?;
    let json = |key: &str| responses.get(key).and_then(|j| serde_json::from_str::<Value>(j).ok()).and_then(|v| v.as_array().cloned()).unwrap_or_default();
    let mut notifications: Vec<(String, String, String)> = threads.into_iter().filter_map(|(id, repo, at)| Some((id, repo?, at))).collect();
    for n in json(NOTIFICATIONS) {
        if let (Some(id), Some(repo), Some(at)) = (n["id"].as_str(), n["repository"]["full_name"].as_str(), n["updated_at"].as_str()) {
            notifications.push((id.into(), repo.into(), at.into()));
        }
    }
    let pushed: Vec<String> = json(PUSHED).iter().filter(|r| r["archived"].as_bool() != Some(true)).filter_map(|r| r["full_name"].as_str().map(str::to_string)).collect();
    let cutoff = time::OffsetDateTime::now_utc().unix_timestamp() - RECENT_DAYS * 86_400;
    Ok(rank(notifications, pushed, cached, &registered, cutoff))
}

/// `notifications`: (thread id, repo, RFC 3339 update time), possibly duplicated across sources.
fn rank(notifications: Vec<(String, String, String)>, pushed: Vec<String>, cached: Vec<String>, registered: &HashSet<String>, cutoff: i64) -> Vec<CloneSuggestion> {
    let at = |s: &str| time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).map(|t| t.unix_timestamp()).unwrap_or(0);
    let mut seen_threads = HashSet::new();
    // Lowercase repo -> (display name, recent thread count, latest update).
    let mut active: HashMap<String, (String, u32, i64)> = HashMap::new();
    let mut names = Vec::new();
    for (id, repo, updated) in notifications {
        names.push(repo.clone());
        let t = at(&updated);
        if t < cutoff || !seen_threads.insert(id) {
            continue;
        }
        let entry = active.entry(repo.to_lowercase()).or_insert((repo, 0, 0));
        entry.1 += 1;
        entry.2 = entry.2.max(t);
    }
    let mut first: Vec<_> = active.into_values().collect();
    first.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)).then(a.0.to_lowercase().cmp(&b.0.to_lowercase())));
    let mut rest: Vec<String> = names.into_iter().chain(cached).collect();
    rest.sort_by_key(|r| r.to_lowercase());
    let mut out = Vec::new();
    let mut taken = HashSet::new();
    let candidates = first.into_iter().map(|(r, n, _)| (r, n)).chain(pushed.into_iter().map(|r| (r, 0))).chain(rest.into_iter().map(|r| (r, 0)));
    for (repo, notifications) in candidates {
        let key = repo.to_lowercase();
        if parse_repo(&repo).is_ok() && !registered.contains(&key) && taken.insert(key) {
            out.push(CloneSuggestion { repo, notifications });
        }
    }
    out
}

/// Fetches the latest notifications and the owner's recently pushed repositories, best effort (GitHub may be
/// signed out or unreachable), then returns `suggestions`.
pub async fn refresh_suggestions(app: &App) -> Result<Vec<CloneSuggestion>> {
    let fetch = |path: &'static str| async move {
        if let Err(e) = tokio::time::timeout(Duration::from_secs(15), app.github.request(reqwest::Method::GET, path, None, false)).await.unwrap_or_else(|_| Err(anyhow::anyhow!("timed out"))) {
            tracing::debug!("clone suggestions {path}: {e:#}");
        }
    };
    tokio::join!(fetch(NOTIFICATIONS), fetch(PUSHED));
    suggestions(app).await
}

/// Clones into `<projects_root>/<name>` with `gh repo clone` (its configured protocol and credentials),
/// or `git clone` over HTTPS without a signed-in GitHub CLI. A hidden staging folder keeps Discover from listing a
/// partial clone; no prompt can block the request.
/// On an SSH host, the clone runs there with the host's own `gh` or `git` credentials.
pub async fn clone(app: &App, input: &str, host: Option<&str>) -> Result<Project> {
    let (owner, repo) = parse_repo(input)?;
    let root = crate::projects::projects_root(app, host)?;
    crate::hostfs::create_dir_all(&root).await.with_context(|| format!("create {}", root.display()))?;
    let dest = root.join(&repo);
    if crate::hostfs::exists(&dest).await {
        bail!("{} already exists; use Discover to add it", dest.display());
    }
    let staging = root.join(format!(".{repo}.cloning-{}", &uuid::Uuid::new_v4().to_string()[..8]));
    let full = format!("{owner}/{repo}");
    let staging_s = staging.to_string_lossy().into_owned();
    let gh = match host {
        Some(h) => crate::hosts::conn(h)?.request(crate::hosts::proto::Op::Which { programs: vec!["gh".into()] }).await.ok().and_then(|v| v[0].as_str().map(str::to_string)).is_some(),
        // A signed-out `gh` refuses even public repositories; plain Git still clones those.
        None => app.env.which("gh").is_some_and(|p| p.is_file()) && crate::procs::output(&app.env, "gh", &["auth", "token"], None).await.is_ok(),
    };
    let mut cmd = if gh {
        let mut c = crate::procs::command(&app.env, "gh", Some(&root));
        c.args(["repo", "clone", &full, &staging_s]);
        c
    } else {
        let mut c = crate::procs::command(&app.env, "git", Some(&root));
        c.args(["clone", &format!("https://github.com/{full}.git"), &staging_s]);
        c
    };
    cmd.env("GIT_TERMINAL_PROMPT", "0").env("GH_PROMPT_DISABLED", "1").stdin(crate::exec::Io::Null);
    let result = tokio::time::timeout(Duration::from_secs(15 * 60), cmd.output()).await;
    let ok = match result {
        Ok(Ok(out)) if out.status.success() => Ok(()),
        Ok(Ok(out)) => Err(anyhow::anyhow!("clone failed: {}", String::from_utf8_lossy(&out.stderr).trim())),
        Ok(Err(e)) => Err(anyhow::Error::new(e).context("run the clone command")),
        Err(_) => Err(anyhow::anyhow!("clone did not finish within 15 minutes")),
    };
    if let Err(e) = ok {
        let _ = crate::hostfs::remove_dir_all(&staging).await;
        return Err(e);
    }
    if crate::hostfs::exists(&dest).await {
        let _ = crate::hostfs::remove_dir_all(&staging).await;
        bail!("{} appeared during the clone; use Discover to add it", dest.display());
    }
    crate::hostfs::rename(&staging, &dest).await.with_context(|| format!("move the clone to {}", dest.display()))?;
    crate::projects::register(app, &dest, false).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_recent_notifications_then_pushed_then_cached() {
        let n = |id: &str, repo: &str, at: &str| (id.to_string(), repo.to_string(), at.to_string());
        let cutoff = time::OffsetDateTime::parse("2026-10-01T00:00:00Z", &time::format_description::well_known::Rfc3339).unwrap().unix_timestamp();
        let notifications = vec![
            n("1", "a/one", "2026-10-20T00:00:00Z"),
            n("2", "b/two", "2026-10-21T00:00:00Z"),
            n("3", "b/two", "2026-10-02T00:00:00Z"),
            n("3", "B/Two", "2026-10-02T00:00:00Z"), // Same thread from the inbox cache and the live page.
            n("4", "c/old", "2026-08-01T00:00:00Z"),
            n("5", "d/mine", "2026-10-22T00:00:00Z"),
        ];
        let registered = HashSet::from(["d/mine".to_string()]);
        let out = rank(notifications, vec!["e/pushed".into(), "a/one".into()], vec!["z/cached".into(), "bad".into()], &registered, cutoff);
        let repos: Vec<_> = out.iter().map(|s| (s.repo.as_str(), s.notifications)).collect();
        assert_eq!(repos, [("b/two", 2), ("a/one", 1), ("e/pushed", 0), ("c/old", 0), ("z/cached", 0)]);
    }
}

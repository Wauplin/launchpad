//! Clone a GitHub repository into the projects root and register it (Projects → Clone from GitHub).
use std::time::Duration;

use anyhow::{Context, Result, bail};

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

/// Repositories seen in the GitHub cache (inbox threads and cached items), without the registered ones.
pub async fn suggestions(app: &App) -> Result<Vec<String>> {
    let registered: std::collections::HashSet<String> = crate::projects::list(app).await?.into_iter().filter_map(|p| p.gh_repo).map(|r| r.to_lowercase()).collect();
    let repos: Vec<String> = app
        .cache_db
        .call(|c| {
            let mut stmt = c.prepare("SELECT json_extract(json, '$.repo') AS r FROM inbox_thread WHERE r IS NOT NULL UNION SELECT repo FROM gh_item WHERE repo IS NOT NULL")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await?;
    let mut out: Vec<String> = repos.into_iter().filter(|r| parse_repo(r).is_ok() && !registered.contains(&r.to_lowercase())).collect();
    out.sort_by_key(|r| r.to_lowercase());
    out.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    Ok(out)
}

/// Clones into `<projects_root>/<name>` with `gh repo clone` (its configured protocol and credentials),
/// or `git clone` over HTTPS without the GitHub CLI. A hidden staging folder keeps Discover from listing a
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
        None => app.env.which("gh").is_some_and(|p| p.is_file()),
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

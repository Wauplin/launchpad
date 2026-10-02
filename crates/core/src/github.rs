//! GitHub REST and GraphQL client, with persistent conditional responses and rate limits.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::RwLock;
use ts_rs::TS;

use crate::model::GitHubRef;
use crate::shellenv::UserEnv;
use crate::util::now_ms;

#[derive(Clone)]
pub struct GitHub {
    env: UserEnv,
    db: crate::db::Db,
    pub limits: Arc<RwLock<std::collections::HashMap<String, RateLimit>>>,
    http: reqwest::Client,
    token: Arc<RwLock<Option<String>>>,
    /// Recent items per repository, for `#` suggestions: (fetched_at, items).
    recent: Arc<RwLock<std::collections::HashMap<String, (i64, Vec<GhItem>)>>>,
}

/// An issue or a pull request, as shown in suggestions and context chips.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct GhItem {
    pub repo: String,
    pub number: u64,
    pub title: String,
    /// `issue` or `pr`.
    pub kind: String,
    /// `open`, `closed`, `merged` or `draft`.
    pub state: String,
    pub author: String,
    pub url: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, TS)]
pub struct RateLimit {
    pub remaining: i64,
    pub reset_at: i64,
    pub retry_at: Option<i64>,
}

pub struct GhResponse {
    pub value: Value,
    pub not_modified: bool,
    pub poll_interval: u64,
    pub has_next: bool,
}

impl GitHub {
    pub fn new(env: UserEnv, db: crate::db::Db) -> GitHub {
        GitHub {
            env,
            db,
            limits: Arc::new(RwLock::new(Default::default())),
            http: crate::status::http_client(),
            token: Arc::new(RwLock::new(None)),
            recent: Arc::new(RwLock::new(Default::default())),
        }
    }

    async fn token(&self, refresh: bool) -> Result<String> {
        if !refresh {
            if let Some(t) = self.token.read().await.clone() {
                return Ok(t);
            }
        }
        let t = crate::procs::output(&self.env, "gh", &["auth", "token"], None)
            .await
            .context("gh auth token")?;
        *self.token.write().await = Some(t.clone());
        Ok(t)
    }

    pub async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
        conditional: bool,
    ) -> Result<GhResponse> {
        anyhow::ensure!(
            path.starts_with('/') && !path.starts_with("//"),
            "invalid GitHub path"
        );
        let resource = if path == "/graphql" { "graphql" } else { "core" };
        if let Some(limit) = self.limits.read().await.get(resource) {
            let until = limit.retry_at.unwrap_or(0).max(if limit.remaining == 0 { limit.reset_at } else { 0 });
            anyhow::ensure!(crate::util::now_ms()/1000 >= until, "GitHub {resource} rate limit; retry after {until}");
        }
        let key = path.to_string();
        let cached = if conditional {
            self.db
                .call(move |c| {
                    use rusqlite::OptionalExtension;
                    Ok(c.query_row(
                        "SELECT etag,last_modified,json FROM gh_response WHERE url=?1",
                        [&key],
                        |r| {
                            Ok((
                                r.get::<_, Option<String>>(0)?,
                                r.get::<_, Option<String>>(1)?,
                                r.get::<_, String>(2)?,
                            ))
                        },
                    )
                    .optional()?)
                })
                .await?
        } else {
            None
        };
        for attempt in 0..2 {
            let token = self.token(attempt > 0).await?;
            let mut req = self
                .http
                .request(method.clone(), format!("https://api.github.com{path}"))
                .bearer_auth(&token)
                .header("Accept", "application/vnd.github+json")
                .header("User-Agent", "launchpad")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .timeout(Duration::from_secs(30));
            if let Some(body) = &body {
                req = req.json(body);
            }
            if let Some((etag, modified, _)) = &cached {
                // Notifications explicitly use Last-Modified, including when both headers exist.
                if path.starts_with("/notifications?") {
                    if let Some(m) = modified {
                        req = req.header("If-Modified-Since", m);
                    }
                } else if let Some(e) = etag {
                    req = req.header("If-None-Match", e);
                }
            }
            let res = req.send().await?;
            if res.status() == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 {
                continue;
            }
            let h = res.headers();
            let header = |name: &str| {
                h.get(name)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string)
            };
            let poll_interval = header("x-poll-interval")
                .and_then(|s| s.parse().ok())
                .unwrap_or(60);
            let retry = header("retry-after")
                .and_then(|s| s.parse::<i64>().ok())
                .map(|s| crate::util::now_ms() / 1000 + s);
            if let (Some(resource), Some(remaining), Some(reset)) = (
                header("x-ratelimit-resource"),
                header("x-ratelimit-remaining"),
                header("x-ratelimit-reset"),
            ) {
                self.limits.write().await.insert(
                    resource,
                    RateLimit {
                        remaining: remaining.parse().unwrap_or(0),
                        reset_at: reset.parse().unwrap_or(0),
                        retry_at: retry,
                    },
                );
            }
            let etag = header("etag");
            let modified = header("last-modified");
            let has_next = header("link").is_some_and(|l| l.contains("rel=\"next\""));
            if res.status() == reqwest::StatusCode::NOT_MODIFIED {
                return Ok(GhResponse {
                    value: serde_json::from_str(&cached.context("304 without cached response")?.2)?,
                    not_modified: true,
                    poll_interval,
                    has_next,
                });
            }
            if !res.status().is_success() {
                bail!("GitHub {path}: {}", res.status());
            }
            let bytes = res.bytes().await?;
            let value = if bytes.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&bytes)?
            };
            if method == reqwest::Method::GET {
                let key = path.to_string();
                let json = serde_json::to_string(&value)?;
                self.db.call(move |c| { c.execute("INSERT OR REPLACE INTO gh_response (url,etag,last_modified,json) VALUES (?1,?2,?3,?4)", rusqlite::params![key,etag,modified,json])?; Ok(()) }).await?;
            }
            return Ok(GhResponse {
                value,
                not_modified: false,
                poll_interval,
                has_next,
            });
        }
        bail!("GitHub {path}: unauthorized")
    }

    pub async fn get(&self, path: &str) -> Result<Value> {
        Ok(self
            .request(reqwest::Method::GET, path, None, true)
            .await?
            .value)
    }

    pub async fn graphql(&self, query: String) -> Result<Value> {
        let v = self
            .request(
                reqwest::Method::POST,
                "/graphql",
                Some(serde_json::json!({"query":query})),
                false,
            )
            .await?
            .value;
        // Keep partial data for inaccessible/deleted subjects; report errors if nothing succeeded.
        if v["data"].is_null() {
            bail!("GitHub GraphQL: {}", v["errors"]);
        }
        Ok(v["data"].clone())
    }

    /// Resolves a pasted link (issue, PR, comment) to a context reference with its title.
    pub async fn resolve_link(&self, url: &str) -> Result<GitHubRef> {
        let parsed = parse_link(url).context("not a GitHub issue, pull request or comment link")?;
        let item = self
            .get(&format!("/repos/{}/issues/{}", parsed.repo, parsed.number))
            .await?;
        let is_pr = item.get("pull_request").is_some();
        let kind = match parsed.anchor {
            Anchor::IssueComment => "comment",
            Anchor::ReviewComment => "review_comment",
            Anchor::None => {
                if is_pr {
                    "pr"
                } else {
                    "issue"
                }
            }
        };
        Ok(GitHubRef {
            url: parsed.canonical,
            kind: kind.into(),
            repo: parsed.repo,
            number: Some(parsed.number),
            title: item["title"].as_str().map(str::to_string),
        })
    }

    /// Recently updated issues and PRs of a repository (cached for 5 minutes), filtered by `q`
    /// (a number prefix or words of the title).
    pub async fn suggest(&self, repo: &str, q: &str) -> Result<Vec<GhItem>> {
        let cached = self.recent.read().await.get(repo).cloned();
        let items = match cached {
            Some((at, items)) if now_ms() - at < 5 * 60_000 => items,
            _ => {
                let v = self
                    .get(&format!(
                        "/repos/{repo}/issues?state=all&sort=updated&direction=desc&per_page=100"
                    ))
                    .await?;
                let items: Vec<GhItem> = v
                    .as_array()
                    .map(|a| a.iter().filter_map(|i| item_from_rest(repo, i)).collect())
                    .unwrap_or_default();
                self.recent
                    .write()
                    .await
                    .insert(repo.to_string(), (now_ms(), items.clone()));
                items
            }
        };
        let q = q.trim().trim_start_matches('#').to_lowercase();
        let mut out: Vec<GhItem> = if q.is_empty() {
            items
        } else if q.chars().all(|c| c.is_ascii_digit()) {
            items
                .into_iter()
                .filter(|i| i.number.to_string().starts_with(&q))
                .collect()
        } else {
            let words: Vec<&str> = q.split_whitespace().collect();
            items
                .into_iter()
                .filter(|i| words.iter().all(|w| i.title.to_lowercase().contains(w)))
                .collect()
        };
        // An exact number that is not in the recent list is fetched on demand.
        if out.is_empty() {
            if let Ok(n) = q.parse::<u64>() {
                if let Ok(v) = self.get(&format!("/repos/{repo}/issues/{n}")).await {
                    out.extend(item_from_rest(repo, &v));
                }
            }
        }
        out.truncate(12);
        Ok(out)
    }
}

fn item_from_rest(repo: &str, i: &Value) -> Option<GhItem> {
    let is_pr = i.get("pull_request").is_some();
    let state = if is_pr && i["pull_request"]["merged_at"].is_string() {
        "merged"
    } else if is_pr && i["draft"].as_bool() == Some(true) && i["state"] == "open" {
        "draft"
    } else {
        i["state"].as_str().unwrap_or("open")
    };
    Some(GhItem {
        repo: repo.to_string(),
        number: i["number"].as_u64()?,
        title: i["title"].as_str()?.to_string(),
        kind: if is_pr { "pr" } else { "issue" }.into(),
        state: state.into(),
        author: i["user"]["login"].as_str().unwrap_or("").to_string(),
        url: i["html_url"].as_str()?.to_string(),
        updated_at: i["updated_at"].as_str().unwrap_or("").to_string(),
    })
}

#[derive(Debug, PartialEq)]
enum Anchor {
    None,
    IssueComment,
    ReviewComment,
}

#[derive(Debug)]
struct ParsedLink {
    repo: String,
    number: u64,
    anchor: Anchor,
    canonical: String,
}

/// `https://github.com/o/r/issues/12`, `/pull/12`, `/pull/12/files`, with `#issuecomment-…` or `#discussion_r…`.
fn parse_link(url: &str) -> Option<ParsedLink> {
    let url = url.trim();
    let rest = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))?;
    let (path, fragment) = match rest.split_once('#') {
        Some((p, f)) => (p, Some(f)),
        None => (rest, None),
    };
    let path = path.split('?').next().unwrap_or(path);
    let mut it = path.split('/');
    let (owner, repo, kind, number) = (it.next()?, it.next()?, it.next()?, it.next()?);
    if kind != "issues" && kind != "pull" {
        return None;
    }
    let number: u64 = number.parse().ok()?;
    let anchor = match fragment {
        Some(f) if f.starts_with("issuecomment-") => Anchor::IssueComment,
        Some(f)
            if f.starts_with("discussion_r")
                || f.starts_with("r") && f[1..].chars().all(|c| c.is_ascii_digit()) =>
        {
            Anchor::ReviewComment
        }
        _ => Anchor::None,
    };
    let base = format!("https://github.com/{owner}/{repo}/{kind}/{number}");
    let canonical = match (&anchor, fragment) {
        (Anchor::None, _) | (_, None) => base,
        (_, Some(f)) => format!("{base}#{f}"),
    };
    Some(ParsedLink {
        repo: format!("{owner}/{repo}"),
        number,
        anchor,
        canonical,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links() {
        let p = parse_link("https://github.com/huggingface/huggingface_hub/pull/3420/files?w=1")
            .unwrap();
        assert_eq!(
            (p.repo.as_str(), p.number, p.anchor, p.canonical.as_str()),
            (
                "huggingface/huggingface_hub",
                3420,
                Anchor::None,
                "https://github.com/huggingface/huggingface_hub/pull/3420"
            )
        );
        let p = parse_link("https://github.com/a/b/issues/7#issuecomment-123").unwrap();
        assert_eq!(
            (p.anchor, p.canonical.as_str()),
            (
                Anchor::IssueComment,
                "https://github.com/a/b/issues/7#issuecomment-123"
            )
        );
        assert_eq!(
            parse_link("https://github.com/a/b/pull/7#discussion_r99")
                .unwrap()
                .anchor,
            Anchor::ReviewComment
        );
        assert!(parse_link("https://github.com/a/b/commit/abc").is_none());
        assert!(parse_link("https://example.com/a/b/issues/1").is_none());
    }
}

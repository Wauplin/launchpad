//! Persistent inbox mirror. Sync and mutations share a lock so polling cannot undo a click.
use crate::{App, github::RateLimit, util::now_ms};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tokio::sync::{Mutex, RwLock};
use ts_rs::TS;

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Thread {
    pub id: String,
    pub repo: String,
    pub title: String,
    pub kind: String,
    pub number: Option<u64>,
    pub url: String,
    pub subject_path: Option<String>,
    pub reason: String,
    pub unread: bool,
    pub local_unread: bool,
    pub updated_at: String,
    pub state: Option<String>,
    pub participants: Vec<String>,
    pub participant_count: u64,
    pub review_decision: Option<String>,
    pub checks: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Comment {
    pub author: String,
    pub body: String,
    pub url: String,
    pub created_at: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ThreadPreview {
    pub id: String,
    pub body: String,
    pub author: String,
    pub comments: Vec<Comment>,
    pub fetched_at: i64,
}
#[derive(Clone, Default, Serialize, TS)]
pub struct SyncStatus {
    pub syncing: bool,
    pub last_sync: Option<i64>,
    pub error: Option<String>,
    pub poll_interval: u64,
}
#[derive(Serialize, TS)]
pub struct InboxSnapshot {
    pub threads: Vec<Thread>,
    pub status: SyncStatus,
    pub limits: HashMap<String, RateLimit>,
}
#[derive(Clone)]
pub struct Inbox {
    lock: Arc<Mutex<()>>,
    status: Arc<RwLock<SyncStatus>>,
    full_at: Arc<Mutex<i64>>,
    next_poll: Arc<Mutex<i64>>,
}
impl Default for Inbox {
    fn default() -> Self {
        Self::new()
    }
}
impl Inbox {
    pub fn new() -> Self {
        Self {
            lock: Arc::new(Mutex::new(())),
            status: Arc::new(RwLock::new(SyncStatus::default())),
            full_at: Arc::new(Mutex::new(0)),
            next_poll: Arc::new(Mutex::new(0)),
        }
    }
    pub async fn list(&self, app: &App) -> Result<InboxSnapshot> {
        let mut threads: Vec<Thread> = app
            .cache_db
            .call(|c| {
                let mut q =
                    c.prepare("SELECT json FROM inbox_thread ORDER BY updated_at DESC,id")?;
                let rows = q.query_map([], |r| r.get::<_, String>(0))?;
                rows.map(|s| Ok(serde_json::from_str(&s?)?)).collect()
            })
            .await?;
        let overrides: HashMap<String, (String, String)> = app
            .state_db
            .call(|c| {
                let mut q = c.prepare("SELECT id,updated_at,action FROM inbox_override")?;
                let rows = q.query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        (r.get::<_, String>(1)?, r.get::<_, String>(2)?),
                    ))
                })?;
                Ok(rows.collect::<std::result::Result<_, _>>()?)
            })
            .await?;
        threads.retain_mut(|t| {
            if let Some((_, action)) = overrides.get(&t.id).filter(|(at, _)| at == &t.updated_at) {
                if action == "done" {
                    return false;
                }
                t.local_unread = action == "unread";
                t.unread = t.local_unread;
            }
            true
        });
        Ok(InboxSnapshot {
            threads,
            status: self.status.read().await.clone(),
            limits: app.github.limits.read().await.clone(),
        })
    }
    pub async fn preview(&self, app: &App, id: &str) -> Result<Option<ThreadPreview>> {
        let id = id.to_string();
        app.cache_db
            .call(move |c| {
                use rusqlite::OptionalExtension;
                let s: Option<String> = c
                    .query_row("SELECT json FROM inbox_preview WHERE id=?1", [id], |r| {
                        r.get(0)
                    })
                    .optional()?;
                Ok(s.map(|s| serde_json::from_str(&s)).transpose()?)
            })
            .await
    }
    pub async fn sync(&self, app: &App) -> Result<()> {
        let _lock = self.lock.lock().await;
        let now = now_ms();
        // Even failed polls and manual refreshes obey the minimum polling interval.
        if now < *self.next_poll.lock().await { return Ok(()); }
        *self.next_poll.lock().await = now + self.status.read().await.poll_interval.max(60) as i64 * 1000;
        self.status.write().await.syncing = true;
        changed(app);
        let result = self.sync_inner(app, now).await;
        {
            let mut s = self.status.write().await;
            s.syncing = false;
            s.error = result.as_ref().err().map(|e| format!("{e:#}"));
            if result.is_ok() {
                s.last_sync = Some(now_ms());
            }
        }
        *self.next_poll.lock().await = now_ms() + self.status.read().await.poll_interval.max(60) as i64 * 1000;
        changed(app);
        result
    }
    async fn sync_inner(&self, app: &App, now: i64) -> Result<()> {
        let full = now - *self.full_at.lock().await >= 30 * 60_000;
        let mut incoming = Vec::new();
        let mut page = 1;
        loop {
            let path = format!("/notifications?all=true&per_page=50&page={page}");
            let r = app
                .github
                .request(reqwest::Method::GET, &path, None, !full)
                .await?;
            self.status.write().await.poll_interval = r
                .poll_interval
                .max(app.config.get().github.poll_interval_seconds)
                .max(60);
            if r.not_modified && !full {
                break;
            }
            incoming.extend(
                r.value
                    .as_array()
                    .context("invalid notifications response")?
                    .iter()
                    .filter_map(from_rest),
            );
            if !full || !r.has_next {
                break;
            }
            page += 1;
        }
        let old = self.list(app).await?.threads;
        let old_map: HashMap<_, _> = old.iter().map(|t| (&t.id, t)).collect();
        let changed_threads: Vec<_> = incoming
            .iter()
            .filter(|t| {
                old_map
                    .get(&t.id)
                    .is_none_or(|o| o.updated_at != t.updated_at || o.state.is_none())
            })
            .cloned()
            .collect();
        let saved = incoming.clone();
        let prior_threads: HashMap<_, _> = old.into_iter().map(|t| (t.id.clone(), t)).collect();
        app.cache_db.call(move |c| {
            let tx=c.transaction()?;
            if full {tx.execute("DELETE FROM inbox_thread",[])?;}
            for mut t in saved {
                // Preserve enriched fields when an unchanged notification is returned.
                if let Some(p)=prior_threads.get(&t.id) { if p.updated_at==t.updated_at {t.state=p.state.clone();t.participants=p.participants.clone();t.participant_count=p.participant_count;t.review_decision=p.review_decision.clone();t.checks=p.checks.clone();} }
                tx.execute("INSERT OR REPLACE INTO inbox_thread (id,updated_at,json) VALUES (?1,?2,?3)",rusqlite::params![t.id,t.updated_at,serde_json::to_string(&t)?])?;
            }
            tx.commit()?; Ok(())
        }).await?;
        if full {
            *self.full_at.lock().await = now;
        }
        changed(app);
        // Summary enrichment also fills the preview cache; prioritize likely next clicks.
        let top: Vec<_> = self
            .list(app)
            .await?
            .threads
            .into_iter()
            .filter(|t| t.unread)
            .take(15)
            .collect();
        let mut enrich = changed_threads;
        enrich.sort_by_key(|t| (!t.unread, std::cmp::Reverse(t.updated_at.clone())));
        let mut ids: HashSet<_> = enrich.iter().map(|t| t.id.clone()).collect();
        for t in top {
            if self.preview(app, &t.id).await?.is_none() && ids.insert(t.id.clone()) {
                enrich.push(t);
            }
        }
        for batch in enrich.chunks(20) {
            self.enrich(app, batch).await?;
            changed(app);
        }
        Ok(())
    }
    async fn enrich(&self, app: &App, threads: &[Thread]) -> Result<()> {
        // Releases and commits have REST summaries rather than GraphQL issue/PR fields.
        for t in threads.iter().filter(|t| t.kind == "Release" || t.kind == "Commit") {
            let Some(path) = &t.subject_path else { continue; };
            let value = match app.github.get(path).await {
                Ok(v) => v,
                Err(e) => { tracing::debug!("notification subject unavailable: {e:#}"); continue; }
            };
            let mut t = t.clone();
            if let Some(url) = value["html_url"].as_str() { t.url = url.to_string(); }
            let p = ThreadPreview {
                id: t.id.clone(), author: string(&value["author"], "login"),
                body: value["body"].as_str().or(value["commit"]["message"].as_str()).unwrap_or("").to_string(),
                comments: vec![], fetched_at: now_ms(),
            };
            app.cache_db.call(move |c| {
                c.execute("UPDATE inbox_thread SET json=?2 WHERE id=?1", rusqlite::params![t.id,serde_json::to_string(&t)?])?;
                c.execute("INSERT OR REPLACE INTO inbox_preview VALUES (?1,?2,?3)",rusqlite::params![p.id,p.fetched_at,serde_json::to_string(&p)?])?;
                Ok(())
            }).await?;
        }
        let subjects: Vec<_> = threads
            .iter()
            .filter(|t| t.number.is_some() && (t.kind == "Issue" || t.kind == "PullRequest"))
            .collect();
        if subjects.is_empty() {
            return Ok(());
        }
        let common = "title url body state updatedAt author { login } participants(first:8) { totalCount nodes { login } } comments(last:8) { nodes { body url createdAt author { login } } }";
        let mut query = String::from("query {");
        for (i, t) in subjects.iter().enumerate() {
            let (owner, repo) = t.repo.split_once('/').context("invalid repository")?;
            let field = if t.kind == "PullRequest" {
                "pullRequest"
            } else {
                "issue"
            };
            let extra = if t.kind == "PullRequest" {
                "isDraft merged reviewDecision commits(last:1) { nodes { commit { statusCheckRollup { state } } } }"
            } else {
                ""
            };
            query.push_str(&format!("t{i}: repository(owner:{},name:{}) {{ subject:{field}(number:{}) {{ {common} {extra} }} }}",serde_json::to_string(owner)?,serde_json::to_string(repo)?,t.number.unwrap()));
        }
        query.push('}');
        let data = app.github.graphql(query).await?;
        let mut saves = Vec::new();
        for (i, t) in subjects.into_iter().enumerate() {
            let v = &data[format!("t{i}")]["subject"];
            if v.is_null() {
                continue;
            }
            let mut t = t.clone();
            t.title = string(v, "title");
            t.url = string(v, "url");
            t.state = Some(if v["merged"] == true {
                "merged".into()
            } else if v["isDraft"] == true && v["state"] == "OPEN" {
                "draft".into()
            } else {
                string(v, "state").to_lowercase()
            });
            t.participants = v["participants"]["nodes"]
                .as_array()
                .map(|a| a.iter().map(|p| string(p, "login")).collect())
                .unwrap_or_default();
            t.participant_count = v["participants"]["totalCount"].as_u64().unwrap_or(0);
            t.review_decision = v["reviewDecision"].as_str().map(str::to_string);
            t.checks = v["commits"]["nodes"][0]["commit"]["statusCheckRollup"]["state"]
                .as_str()
                .map(str::to_string);
            let preview = ThreadPreview {
                id: t.id.clone(),
                body: string(v, "body"),
                author: string(&v["author"], "login"),
                fetched_at: now_ms(),
                comments: v["comments"]["nodes"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|c| Comment {
                                author: string(&c["author"], "login"),
                                body: string(c, "body"),
                                url: string(c, "url"),
                                created_at: string(c, "createdAt"),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            };
            saves.push((t, preview));
        }
        app.cache_db.call(move |c|{let tx=c.transaction()?;for (t,p) in saves {
            tx.execute("UPDATE inbox_thread SET json=?2 WHERE id=?1 AND updated_at=?3",rusqlite::params![t.id,serde_json::to_string(&t)?,t.updated_at])?;
            tx.execute("INSERT OR REPLACE INTO inbox_preview (id,fetched_at,json) VALUES (?1,?2,?3)",rusqlite::params![p.id,p.fetched_at,serde_json::to_string(&p)?])?;
        }tx.commit()?;Ok(())}).await
    }
    pub async fn refresh_preview(&self, app: &App, id: &str, force: bool) -> Result<()> {
        let _lock = self.lock.lock().await;
        let t = self
            .list(app)
            .await?
            .threads
            .into_iter()
            .find(|t| t.id == id)
            .context("no such thread")?;
        if !force && self.preview(app, id).await?.is_some_and(|p| now_ms()-p.fetched_at < 60_000) { return Ok(()); }
        self.enrich(app, &[t]).await?;
        changed(app);
        Ok(())
    }
    pub async fn action(&self, app: &App, id: &str, action: &str) -> Result<()> {
        ensure!(
            ["read", "unread", "done", "unsubscribe"].contains(&action),
            "unknown inbox action"
        );
        let _lock = self.lock.lock().await;
        let t = self
            .list(app)
            .await?
            .threads
            .into_iter()
            .find(|t| t.id == id)
            .context("no such thread")?;
        let id = t.id.clone();
        let at = t.updated_at.clone();
        let local = if action == "unsubscribe" {
            "done"
        } else {
            action
        }
        .to_string();
        let previous: Option<(String, String)> = app
            .state_db
            .call(move |c| {
                use rusqlite::OptionalExtension;
                let prior = c
                    .query_row(
                        "SELECT updated_at,action FROM inbox_override WHERE id=?1",
                        [&id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?;
                c.execute(
                    "INSERT OR REPLACE INTO inbox_override VALUES (?1,?2,?3)",
                    rusqlite::params![id, at, local],
                )?;
                Ok(prior)
            })
            .await?;
        changed(app);
        let result = async {
            let path = format!("/notifications/threads/{}", t.id);
            if action == "unsubscribe" {
                app.github
                    .request(
                        reqwest::Method::PUT,
                        &format!("{path}/subscription"),
                        Some(json!({"ignored":true})),
                        false,
                    )
                    .await?;
            }
            if action != "unread" {
                let method = if action == "read" {
                    reqwest::Method::PATCH
                } else {
                    reqwest::Method::DELETE
                };
                app.github.request(method, &path, None, false).await?;
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if result.is_err() {
            let id = t.id;
            app.state_db
                .call(move |c| {
                    c.execute("DELETE FROM inbox_override WHERE id=?1", [&id])?;
                    if let Some((at, a)) = previous {
                        c.execute(
                            "INSERT INTO inbox_override VALUES (?1,?2,?3)",
                            rusqlite::params![id, at, a],
                        )?;
                    }
                    Ok(())
                })
                .await?;
            changed(app);
        }
        result
    }
    pub async fn reply(&self, app: &App, id: &str, body: &str) -> Result<()> {
        ensure!(!body.trim().is_empty(), "reply is empty");
        let t = self
            .list(app)
            .await?
            .threads
            .into_iter()
            .find(|t| t.id == id)
            .context("no such thread")?;
        ensure!(
            t.kind == "Issue" || t.kind == "PullRequest",
            "this notification does not support replies"
        );
        app.github
            .request(
                reqwest::Method::POST,
                &format!(
                    "/repos/{}/issues/{}/comments",
                    t.repo,
                    t.number.context("missing subject number")?
                ),
                Some(json!({"body":body})),
                false,
            )
            .await?;
        // Posting succeeded even if fetching the updated preview fails.
        if let Err(e) = self.refresh_preview(app, id, true).await {
            tracing::warn!("refresh after reply: {e:#}");
        }
        Ok(())
    }
}
fn string(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or("").to_string()
}
fn from_rest(v: &Value) -> Option<Thread> {
    let repo = v["repository"]["full_name"].as_str()?.to_string();
    let kind = string(&v["subject"], "type");
    let subject = v["subject"]["url"].as_str().unwrap_or("");
    let number = if kind == "Issue" || kind == "PullRequest" {
        subject.rsplit('/').next().and_then(|n| n.parse().ok())
    } else {
        None
    };
    let url = match number {
        Some(n) => format!(
            "https://github.com/{repo}/{}/{n}",
            if kind == "PullRequest" {
                "pull"
            } else {
                "issues"
            }
        ),
        None if kind == "Release" => format!("https://github.com/{repo}/releases"),
        None => subject
            .strip_prefix("https://api.github.com/repos/")
            .map(|s| format!("https://github.com/{}", s.replace("/commits/", "/commit/")))
            .unwrap_or_else(|| format!("https://github.com/{repo}")),
    };
    Some(Thread {
        id: v["id"].as_str()?.into(),
        repo,
        title: string(&v["subject"], "title"),
        kind,
        number,
        url,
        subject_path: subject.strip_prefix("https://api.github.com").filter(|p| p.starts_with("/repos/")).map(str::to_string),
        reason: string(v, "reason"),
        unread: v["unread"] == true,
        local_unread: false,
        updated_at: string(v, "updated_at"),
        state: None,
        participants: vec![],
        participant_count: 0,
        review_decision: None,
        checks: None,
    })
}
fn changed(app: &App) {
    app.bus.publish("inbox", "updated", json!({}));
}
pub fn spawn(app: &App) {
    let app = app.clone();
    tokio::spawn(async move {
        loop {
            if let Err(e) = app.inbox.sync(&app).await {
                tracing::warn!("inbox sync: {e:#}");
            }
            let seconds = app.inbox.status.read().await.poll_interval.max(60);
            tokio::time::sleep(std::time::Duration::from_secs(seconds)).await;
        }
    });
}

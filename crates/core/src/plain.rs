//! Plain and asset folders: files without Git, stored by Home.
//!
//! A store holds a tree (path → content hash) in `state.db` and its contents as content-addressed blobs in
//! `<data_dir>/blobs/sha256/`, which Home backups upload by hash. Every overwrite keeps the previous version and
//! every delete goes to the trash (`plain_history`, 30 days), with Restore. One session writes a store at a
//! time: Home holds its lock.
//!
//! - A plain folder is a folder record (`Project.plain`) whose store has the folder's ID. Runners keep a copy
//!   in `<data_dir>/plain/<id>`, synced down before a session starts and up at the end of every turn.
//! - An asset folder belongs to a Git folder: `launchpad assets create <path>` creates a store and maps the
//!   path in the committed `.launchpad/assets.toml`; worktrees of that branch sync it like a plain folder.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ts_rs::TS;

use crate::App;
use crate::util::now_ms;

/// Previous versions and the trash are kept this long.
const KEEP_MS: i64 = 30 * 24 * 3600 * 1000;
/// Header naming the session of a runner's request.
pub const SESSION_HEADER: &str = "X-Launchpad-Session";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, TS)]
pub struct Entry {
    pub path: String,
    pub sha256: String,
    #[ts(type = "number")]
    pub size: u64,
    pub executable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Tree {
    #[ts(type = "number")]
    pub version: i64,
    pub files: Vec<Entry>,
}

/// One change of a store: `sha256` is the content before (None: the file was created), `new_sha256` after
/// (None: deleted, now in the trash).
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Change {
    #[ts(type = "number")]
    pub id: i64,
    pub path: String,
    pub sha256: Option<String>,
    pub new_sha256: Option<String>,
    #[ts(type = "number")]
    pub at: i64,
    pub session_id: Option<String>,
    /// Knowledge base changes: who wrote it, and the lines added and removed.
    pub author: Option<crate::kb::Author>,
    pub added: Option<u32>,
    pub removed: Option<u32>,
}

/// What a knowledge base change records next to a plain change (`plain_history.meta`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Meta {
    pub author: crate::kb::Author,
    pub added: Option<u32>,
    pub removed: Option<u32>,
}

/// A change sent by a runner: the new content of `path`, or None to delete it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Update {
    pub path: String,
    pub sha256: Option<String>,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub executable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Asset {
    pub id: String,
    /// The Git folder it belongs to.
    pub project_id: String,
    pub name: String,
    #[ts(type = "number")]
    pub created_at: i64,
    /// Moved to the trash by the cleanup of unused asset folders (its files stay restorable 30 days).
    #[serde(default)]
    #[ts(type = "number | null")]
    pub trashed_at: Option<i64>,
    /// Restored from the trash: counts as a change, so the cleanup waits 14 more days.
    #[serde(default)]
    #[ts(type = "number | null")]
    pub restored_at: Option<i64>,
}

/// An asset folder on the folder page.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct AssetInfo {
    pub asset: Asset,
    #[ts(type = "number")]
    pub files: u64,
    #[ts(type = "number")]
    pub size: u64,
    /// The last change of its files (or its creation).
    #[ts(type = "number")]
    pub last_change: i64,
    /// A branch, kept tip or note still names it, or a session holds it.
    pub in_use: bool,
}

/// Unused asset folders move to the trash after this long.
const UNUSED_MS: i64 = 14 * 24 * 3600 * 1000;

fn blob_dir(app: &App) -> PathBuf {
    app.paths.data_dir.join("blobs/sha256")
}

pub fn blob_path(app: &App, sha: &str) -> Result<PathBuf> {
    ensure!(sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()), "invalid content hash");
    Ok(blob_dir(app).join(sha))
}

pub(crate) fn check_path(path: &str) -> Result<()> {
    ensure!(!path.is_empty() && !path.starts_with('/') && !path.contains('\\') && path.split('/').all(|c| !c.is_empty() && c != "." && c != ".."), "invalid path {path:?}");
    ensure!(!path.split('/').any(|c| c == ".git"), "Git data is never stored in a plain folder");
    Ok(())
}

/// Home: the store exists (a plain folder, an asset folder or a workspace's knowledge base).
async fn ensure_store(app: &App, id: &str) -> Result<()> {
    if let Some(ws) = crate::kb::workspace_of(id) {
        crate::workspace::get(app, ws).await?;
        return Ok(());
    }
    if let Some(bot) = crate::bots::id_of(id) {
        ensure!(crate::bots::exists(app, bot).await?, "no such bot");
        return Ok(());
    }
    let id = id.to_string();
    let known = app.state_db.call(move |c| {
        let plain: bool = c.query_row("SELECT 1 FROM project WHERE id = ?1 AND plain = 1", [&id], |_| Ok(true)).optional()?.unwrap_or(false);
        Ok(plain || c.query_row("SELECT 1 FROM plain_asset WHERE id = ?1", [&id], |_| Ok(true)).optional()?.unwrap_or(false))
    }).await?;
    ensure!(known, "no such plain or asset folder");
    Ok(())
}

pub async fn tree(app: &App, id: &str) -> Result<Tree> {
    ensure_store(app, id).await?;
    let id = id.to_string();
    app.state_db.call(move |c| {
        let version: i64 = c.query_row("SELECT version FROM plain_state WHERE store = ?1", [&id], |r| r.get(0)).optional()?.unwrap_or(0);
        let mut st = c.prepare("SELECT path, sha256, size, executable FROM plain_file WHERE store = ?1 ORDER BY path")?;
        let files = st.query_map([&id], |r| Ok(Entry { path: r.get(0)?, sha256: r.get(1)?, size: r.get::<_, i64>(2)? as u64, executable: r.get::<_, i64>(3)? != 0 }))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Tree { version, files })
    }).await
}

/// Home: a blob, fetched from the backup destination when this disk lost it (a restored Home).
pub async fn read_blob(app: &App, sha: &str) -> Result<Vec<u8>> {
    let path = blob_path(app, sha)?;
    if let Ok(data) = tokio::fs::read(&path).await {
        return Ok(data);
    }
    let data = crate::home_backup::fetch_blob(app, sha).await.with_context(|| format!("content {sha} is missing"))?;
    write_blob(app, sha, &data).await?;
    Ok(data)
}

/// Stores a blob after checking its hash.
pub async fn write_blob(app: &App, sha: &str, data: &[u8]) -> Result<()> {
    ensure!(hex::encode(Sha256::digest(data)) == sha, "content does not match its hash");
    let path = blob_path(app, sha)?;
    if path.exists() {
        return Ok(());
    }
    tokio::fs::create_dir_all(blob_dir(app)).await?;
    let tmp = path.with_extension(format!("partial-{}", uuid::Uuid::new_v4()));
    tokio::fs::write(&tmp, data).await?;
    tokio::fs::rename(&tmp, &path).await?;
    Ok(())
}

/// Home: applies a runner's changes. The session must hold the store's lock. Returns the new version.
pub async fn commit(app: &App, id: &str, session: Option<&str>, updates: Vec<Update>) -> Result<i64> {
    ensure_store(app, id).await?;
    ensure!(crate::kb::workspace_of(id).is_none(), "the knowledge base is written with launchpad kb");
    ensure!(crate::bots::id_of(id).is_none(), "a bot's files change in its Settings and Files tabs");
    if let Some(s) = session {
        let holder = lock_holder(app, id).await?;
        ensure!(holder.as_deref() == Some(s), "this session does not hold the folder's lock");
    }
    record(app, id, session, updates.into_iter().map(|u| (u, None)).collect()).await
}

/// Home: applies changes, each with its knowledge base record. Callers checked the lock.
pub(crate) async fn record(app: &App, id: &str, session: Option<&str>, updates: Vec<(Update, Option<Meta>)>) -> Result<i64> {
    for (u, _) in &updates {
        check_path(&u.path)?;
        if let Some(sha) = &u.sha256 {
            ensure!(blob_path(app, sha)?.exists(), "content of {} was not uploaded", u.path);
        }
    }
    let (id2, session) = (id.to_string(), session.map(str::to_string));
    let at = now_ms();
    let version = app.state_db.call(move |c| {
        let tx = c.transaction()?;
        for (u, meta) in updates {
            let meta = meta.map(|m| serde_json::to_string(&m)).transpose().map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            let old: Option<String> = tx.query_row("SELECT sha256 FROM plain_file WHERE store = ?1 AND path = ?2", params![id2, u.path], |r| r.get(0)).optional()?;
            if old == u.sha256 {
                continue;
            }
            match &u.sha256 {
                Some(sha) => tx.execute("INSERT INTO plain_file (store, path, sha256, size, executable) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT (store, path) DO UPDATE SET sha256 = ?3, size = ?4, executable = ?5", params![id2, u.path, sha, u.size as i64, u.executable as i64])?,
                None => tx.execute("DELETE FROM plain_file WHERE store = ?1 AND path = ?2", params![id2, u.path])?,
            };
            tx.execute("INSERT INTO plain_history (store, path, sha256, new_sha256, at, session_id, meta) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)", params![id2, u.path, old, u.sha256, at, session, meta])?;
        }
        tx.execute("INSERT INTO plain_state (store, version) VALUES (?1, 1) ON CONFLICT (store) DO UPDATE SET version = version + 1", [&id2])?;
        let v: i64 = tx.query_row("SELECT version FROM plain_state WHERE store = ?1", [&id2], |r| r.get(0))?;
        tx.commit()?;
        Ok(v)
    }).await?;
    match crate::kb::workspace_of(id) {
        Some(ws) => app.bus.publish("kb", "changed", serde_json::json!({"workspace_id": ws, "version": version})),
        None => app.bus.publish("plain", "changed", serde_json::json!({"id": id, "version": version})),
    }
    Ok(version)
}

/// Previous versions and deleted files of the last 30 days, newest first.
pub async fn history(app: &App, id: &str) -> Result<Vec<Change>> {
    let id = id.to_string();
    let cutoff = now_ms() - KEEP_MS;
    app.state_db.call(move |c| {
        let mut st = c.prepare("SELECT id, path, sha256, new_sha256, at, session_id, meta FROM plain_history WHERE store = ?1 AND at >= ?2 ORDER BY id DESC LIMIT 500")?;
        let rows = st
            .query_map(params![id, cutoff], |r| {
                let meta: Option<Meta> = r.get::<_, Option<String>>(6)?.and_then(|m| serde_json::from_str(&m).ok());
                Ok(Change { id: r.get(0)?, path: r.get(1)?, sha256: r.get(2)?, new_sha256: r.get(3)?, at: r.get(4)?, session_id: r.get(5)?, added: meta.as_ref().and_then(|m| m.added), removed: meta.as_ref().and_then(|m| m.removed), author: meta.map(|m| m.author) })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }).await
}

/// Restore: puts back the content a change replaced or deleted, as a new change (itself restorable). Refused
/// while a session holds the folder.
pub async fn restore(app: &App, id: &str, change: i64) -> Result<i64> {
    ensure!(lock_holder(app, id).await?.is_none(), "an agent works in this folder; restore after it is archived");
    let c = history(app, id).await?.into_iter().find(|c| c.id == change).context("this version is no longer kept")?;
    let sha = c.sha256.context("this change created the file: there is no earlier version")?;
    let size = tokio::fs::metadata(blob_path(app, &sha)?).await.map(|m| m.len()).unwrap_or(0);
    commit(app, id, None, vec![Update { path: c.path, sha256: Some(sha), size, executable: false }]).await
}

pub async fn lock_holder(app: &App, id: &str) -> Result<Option<String>> {
    let id = id.to_string();
    app.state_db.call(move |c| Ok(c.query_row("SELECT session_id FROM plain_lock WHERE store = ?1", [id], |r| r.get(0)).optional()?)).await
}

/// Home: the session takes the store's lock. A lock of a session that no longer exists, or that ended, is
/// taken over.
pub async fn lock(app: &App, id: &str, session: &str) -> Result<()> {
    ensure_store(app, id).await?;
    if let Some(holder) = lock_holder(app, id).await?
        && holder != session
        && let Some(s) = app.sessions.get(&holder)
        && s.status != crate::model::SessionStatus::Closed
    {
        bail!("{} is in use by the agent \"{}\"; archive it first (one agent at a time)", folder_name(app, id).await, s.title);
    }
    let (id, session) = (id.to_string(), session.to_string());
    app.state_db.call(move |c| {
        c.execute("INSERT INTO plain_lock (store, session_id, at) VALUES (?1, ?2, ?3) ON CONFLICT (store) DO UPDATE SET session_id = ?2, at = ?3", params![id, session, now_ms()])?;
        Ok(())
    }).await
}

pub async fn unlock(app: &App, id: &str, session: &str) -> Result<()> {
    let (id, session) = (id.to_string(), session.to_string());
    app.state_db.call(move |c| Ok(c.execute("DELETE FROM plain_lock WHERE store = ?1 AND session_id = ?2", params![id, session])?)).await?;
    Ok(())
}

async fn folder_name(app: &App, id: &str) -> String {
    if let Ok(Some(p)) = crate::projects::get(app, id).await {
        return p.name;
    }
    assets(app, None).await.ok().and_then(|a| a.into_iter().find(|a| a.id == id)).map(|a| a.name).unwrap_or_else(|| "This folder".into())
}

/// Home: a new asset folder of a Git folder.
pub async fn create_asset(app: &App, project_id: &str, name: &str) -> Result<Asset> {
    let p = crate::projects::get(app, project_id).await?.context("no such folder")?;
    ensure!(p.home_git, "asset folders belong to Git folders");
    let a = Asset { id: uuid::Uuid::new_v4().to_string(), project_id: project_id.into(), name: name.trim().chars().take(120).collect(), created_at: now_ms(), trashed_at: None, restored_at: None };
    let json = serde_json::to_string(&a)?;
    let (id, pid) = (a.id.clone(), a.project_id.clone());
    app.state_db.call(move |c| Ok(c.execute("INSERT INTO plain_asset (id, project_id, json) VALUES (?1, ?2, ?3)", params![id, pid, json])?)).await?;
    Ok(a)
}

pub async fn assets(app: &App, project_id: Option<&str>) -> Result<Vec<Asset>> {
    let rows: Vec<String> = app.state_db.call(|c| {
        let mut st = c.prepare("SELECT json FROM plain_asset")?;
        let rows = st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }).await?;
    Ok(rows.iter().filter_map(|j| serde_json::from_str::<Asset>(j).ok()).filter(|a| project_id.is_none_or(|p| a.project_id == p)).collect())
}

async fn save_asset(app: &App, a: &Asset) -> Result<()> {
    let (id, json) = (a.id.clone(), serde_json::to_string(a)?);
    app.state_db.call(move |c| Ok(c.execute("UPDATE plain_asset SET json = ?2 WHERE id = ?1", params![id, json])?)).await?;
    Ok(())
}

/// Home: the asset folders a branch, a kept tip or a knowledge base note still names.
async fn referenced_assets(app: &App) -> Result<std::collections::HashSet<String>> {
    let mut ids = std::collections::HashSet::new();
    let assets = assets(app, None).await?;
    let projects: std::collections::HashSet<String> = assets.iter().map(|a| a.project_id.clone()).collect();
    for pid in projects {
        let dir = crate::home_git::mirror(app, &pid);
        if !dir.join("HEAD").exists() {
            continue;
        }
        let refs = crate::home_git::git_in(app, &dir, &["for-each-ref", "--format=%(objectname)", "refs/heads", "refs/kept"], 60).await?;
        let mut commits: Vec<&str> = refs.lines().filter(|l| !l.is_empty()).collect();
        commits.sort();
        commits.dedup();
        for c in commits {
            if let Ok(text) = crate::home_git::git_in(app, &dir, &["cat-file", "-p", &format!("{c}:.launchpad/assets.toml")], 30).await
                && let Ok(table) = toml::from_str::<toml::Table>(&text)
            {
                ids.extend(table.values().filter_map(|v| v.as_str().map(str::to_string)));
            }
        }
    }
    // Notes may name an asset folder by its ID.
    let notes: Vec<String> = app.state_db.call(|c| {
        let mut st = c.prepare("SELECT sha256 FROM plain_file WHERE store LIKE 'kb-%' AND size < 1048576")?;
        let rows = st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }).await?;
    for sha in notes {
        if let Ok(data) = tokio::fs::read(blob_path(app, &sha)?).await {
            let text = String::from_utf8_lossy(&data);
            ids.extend(assets.iter().filter(|a| text.contains(&a.id)).map(|a| a.id.clone()));
        }
    }
    Ok(ids)
}

/// Home: the asset folders of a Git folder, with their size and whether something still uses them.
pub async fn asset_infos(app: &App, project_id: &str) -> Result<Vec<AssetInfo>> {
    let used = referenced_assets(app).await?;
    let mut out = vec![];
    for a in assets(app, Some(project_id)).await? {
        let id = a.id.clone();
        let (files, size, last): (i64, i64, Option<i64>) = app.state_db.call(move |c| {
            let (n, s): (i64, i64) = c.query_row("SELECT count(*), coalesce(sum(size), 0) FROM plain_file WHERE store = ?1", [&id], |r| Ok((r.get(0)?, r.get(1)?)))?;
            let last: Option<i64> = c.query_row("SELECT max(at) FROM plain_history WHERE store = ?1", [&id], |r| r.get(0))?;
            Ok((n, s, last))
        }).await?;
        let locked = lock_holder(app, &a.id).await?.is_some();
        out.push(AssetInfo { in_use: locked || used.contains(&a.id), files: files as u64, size: size as u64, last_change: last.unwrap_or(a.created_at).max(a.created_at).max(a.restored_at.unwrap_or(0)), asset: a });
    }
    out.sort_by_key(|i| std::cmp::Reverse(i.last_change));
    Ok(out)
}

/// Home: asset folders that nothing names and nobody changed for 14 days go to the trash: their files are
/// deleted (restorable for 30 days), then the record goes after 30 days.
pub async fn clean_assets(app: &App) -> Result<()> {
    let now = now_ms();
    let mut projects: Vec<String> = assets(app, None).await?.into_iter().map(|a| a.project_id).collect();
    projects.sort();
    projects.dedup();
    for pid in projects {
        for info in asset_infos(app, &pid).await? {
            let mut a = info.asset;
            match a.trashed_at {
                Some(t) if t < now - KEEP_MS => {
                    let id = a.id.clone();
                    app.state_db.call(move |c| Ok(c.execute("DELETE FROM plain_asset WHERE id = ?1", [id])?)).await?;
                }
                Some(_) => {}
                None if !info.in_use && info.last_change < now - UNUSED_MS => {
                    let files = tree(app, &a.id).await?.files;
                    record(app, &a.id, None, files.into_iter().map(|e| (Update { path: e.path, sha256: None, size: 0, executable: false }, None)).collect()).await?;
                    a.trashed_at = Some(now);
                    save_asset(app, &a).await?;
                    tracing::info!("asset folder {} unused for 14 days: moved to the trash", a.name);
                }
                None => {}
            }
        }
    }
    Ok(())
}

/// Home: puts back the files of an asset folder moved to the trash.
pub async fn untrash_asset(app: &App, id: &str) -> Result<i64> {
    let mut a = assets(app, None).await?.into_iter().find(|a| a.id == id).context("no such asset folder")?;
    let at = a.trashed_at.context("this asset folder is not in the trash")?;
    let deleted: Vec<(String, String)> = {
        let id = id.to_string();
        app.state_db.call(move |c| {
            let mut st = c.prepare("SELECT path, sha256 FROM plain_history WHERE store = ?1 AND new_sha256 IS NULL AND sha256 IS NOT NULL AND at >= ?2 ORDER BY id")?;
            let rows = st.query_map(params![id, at], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        }).await?
    };
    let mut updates = vec![];
    for (path, sha) in deleted {
        let size = tokio::fs::metadata(blob_path(app, &sha)?).await.map(|m| m.len()).unwrap_or(0);
        updates.push((Update { path, sha256: Some(sha), size, executable: false }, None));
    }
    let v = record(app, id, None, updates).await?;
    a.trashed_at = None;
    a.restored_at = Some(now_ms());
    save_asset(app, &a).await?;
    Ok(v)
}

/// Hashes of every blob a store still needs: current files and the versions of the last 30 days.
pub async fn live_blobs(app: &App) -> Result<Vec<String>> {
    let cutoff = now_ms() - KEEP_MS;
    app.state_db.call(move |c| {
        let mut st = c.prepare("SELECT sha256 FROM plain_file UNION SELECT sha256 FROM plain_history WHERE sha256 IS NOT NULL AND at >= ?1")?;
        let rows = st.query_map([cutoff], |r| r.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }).await
}

/// Deletes history older than 30 days and blobs nothing needs any more.
pub async fn prune(app: &App) -> Result<()> {
    let cutoff = now_ms() - KEEP_MS;
    app.state_db.call(move |c| Ok(c.execute("DELETE FROM plain_history WHERE at < ?1", [cutoff])?)).await?;
    let live: std::collections::HashSet<String> = live_blobs(app).await?.into_iter().collect();
    let Ok(mut rd) = tokio::fs::read_dir(blob_dir(app)).await else { return Ok(()) };
    while let Some(e) = rd.next_entry().await? {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.len() == 64 && !live.contains(&name) {
            let _ = tokio::fs::remove_file(e.path()).await;
        }
    }
    Ok(())
}

pub fn spawn(app: &App) {
    if !crate::home_git::here(app) {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(6 * 3600));
        loop {
            tick.tick().await;
            if let Err(e) = clean_assets(&app).await {
                tracing::warn!("asset folders cleanup: {e:#}");
            }
            if let Err(e) = prune(&app).await {
                tracing::warn!("plain folders prune: {e:#}");
            }
        }
    });
}

// ---------------------------------------------------------------- runner side

/// Where this runner keeps its copy of a plain folder.
pub fn copy_dir(app: &App, id: &str) -> PathBuf {
    app.paths.data_dir.join("plain").join(id)
}

/// The last tree this copy synced, kept outside the copy.
fn state_file(app: &App, dir: &Path) -> PathBuf {
    let key = hex::encode(Sha256::digest(dir.to_string_lossy().as_bytes()));
    app.paths.data_dir.join("plain-state").join(format!("{}.json", &key[..32]))
}

#[derive(Default, Serialize, Deserialize)]
struct Synced {
    version: i64,
    files: BTreeMap<String, String>,
}

/// Calls Home: in process on Home and `all`, over HTTP with the runner credential on a runner.
pub(crate) mod home {
    use super::*;

    fn client() -> Result<reqwest::Client> {
        Ok(reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(30)).timeout(std::time::Duration::from_secs(1800)).build()?)
    }

    fn request(app: &App, method: reqwest::Method, path: &str, session: Option<&str>) -> Result<reqwest::RequestBuilder> {
        let link = crate::home_link::load(&app.paths).context("this runner is not connected to a Home")?;
        let mut r = client()?.request(method, format!("{}{path}", link.url.trim_end_matches('/'))).header("x-launchpad-runner", link.credential());
        if let Some(t) = crate::home_link::space_token(&link.url) {
            r = r.bearer_auth(t);
        }
        if let Some(s) = session {
            r = r.header(SESSION_HEADER, s);
        }
        Ok(r)
    }

    async fn json(r: reqwest::RequestBuilder) -> Result<serde_json::Value> {
        let res = r.send().await?;
        let status = res.status();
        let v: serde_json::Value = res.json().await.unwrap_or_default();
        ensure!(status.is_success(), "Home: {}", v["error"].as_str().unwrap_or(status.as_str()));
        Ok(v)
    }

    pub async fn tree(app: &App, id: &str) -> Result<Tree> {
        if crate::home_git::here(app) {
            return super::tree(app, id).await;
        }
        Ok(serde_json::from_value(json(request(app, reqwest::Method::GET, &format!("/api/plain/{id}/tree"), None)?).await?)?)
    }

    pub async fn blob(app: &App, id: &str, sha: &str) -> Result<Vec<u8>> {
        if crate::home_git::here(app) {
            return super::read_blob(app, sha).await;
        }
        let res = request(app, reqwest::Method::GET, &format!("/api/plain/{id}/blob/{sha}"), None)?.send().await?;
        ensure!(res.status().is_success(), "Home could not send {sha}");
        let data = res.bytes().await?.to_vec();
        ensure!(hex::encode(Sha256::digest(&data)) == sha, "content from Home does not match its hash");
        Ok(data)
    }

    pub async fn upload(app: &App, id: &str, sha: &str, data: Vec<u8>) -> Result<()> {
        if crate::home_git::here(app) {
            return super::write_blob(app, sha, &data).await;
        }
        json(request(app, reqwest::Method::PUT, &format!("/api/plain/{id}/blob/{sha}"), None)?.body(data)).await.map(|_| ())
    }

    pub async fn commit(app: &App, id: &str, session: &str, updates: Vec<Update>) -> Result<i64> {
        if crate::home_git::here(app) {
            return super::commit(app, id, Some(session), updates).await;
        }
        let v = json(request(app, reqwest::Method::POST, &format!("/api/plain/{id}/commit"), Some(session))?.json(&updates)).await?;
        v["version"].as_i64().context("version missing")
    }

    pub async fn lock(app: &App, id: &str, session: &str, take: bool) -> Result<()> {
        if crate::home_git::here(app) {
            return if take { super::lock(app, id, session).await } else { super::unlock(app, id, session).await };
        }
        json(request(app, reqwest::Method::POST, &format!("/api/plain/{id}/{}", if take { "lock" } else { "unlock" }), Some(session))?).await.map(|_| ())
    }
}

pub(crate) async fn hash_file(path: &Path) -> Result<(String, u64, bool)> {
    let data = tokio::fs::read(path).await?;
    #[cfg(unix)]
    let exec = {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::metadata(path).await?.permissions().mode() & 0o111 != 0
    };
    #[cfg(not(unix))]
    let exec = false;
    Ok((hex::encode(Sha256::digest(&data)), data.len() as u64, exec))
}

/// Files of a copy (no `.git`, no symlinks), relative paths with `/`.
pub fn walk(root: &Path) -> Vec<String> {
    let mut out = vec![];
    for e in ignore::WalkBuilder::new(root).hidden(false).git_ignore(false).git_exclude(false).git_global(false).ignore(false).parents(false).filter_entry(|e| e.file_name() != ".git").build().flatten() {
        if e.file_type().is_some_and(|t| t.is_file())
            && let Ok(rel) = e.path().strip_prefix(root)
        {
            out.push(rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/"));
        }
    }
    out.sort();
    out
}

/// Takes the lock and brings the copy to Home's version: files Home changed are written, files Home no longer
/// has are removed. A local Git repository records the start, so Changes compares with it.
pub async fn sync_down(app: &App, id: &str, dir: &Path, session: &str) -> Result<()> {
    home::lock(app, id, session, true).await?;
    let tree = home::tree(app, id).await?;
    tokio::fs::create_dir_all(dir).await?;
    let state = state_file(app, dir);
    let mut synced: Synced = tokio::fs::read(&state).await.ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let wanted: BTreeMap<&str, &Entry> = tree.files.iter().map(|e| (e.path.as_str(), e)).collect();
    for rel in walk(dir) {
        if !wanted.contains_key(rel.as_str()) && synced.files.contains_key(&rel) {
            let _ = tokio::fs::remove_file(dir.join(&rel)).await;
        }
    }
    for e in &tree.files {
        check_path(&e.path)?;
        let path = dir.join(&e.path);
        if path.exists() && hash_file(&path).await.is_ok_and(|(h, _, _)| h == e.sha256) {
            continue;
        }
        let data = home::blob(app, id, &e.sha256).await?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&path, data).await?;
        #[cfg(unix)]
        if e.executable {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).await?;
        }
    }
    synced = Synced { version: tree.version, files: tree.files.iter().map(|e| (e.path.clone(), e.sha256.clone())).collect() };
    if let Some(parent) = state.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&state, serde_json::to_vec(&synced)?).await?;
    baseline(app, dir).await;
    Ok(())
}

/// A local Git repository of the copy, committed at the start of the session (never synced).
async fn baseline(app: &App, dir: &Path) {
    let run = |args: &'static [&'static str]| async move {
        let mut cmd = crate::procs::command(&app.env, "git", Some(dir));
        cmd.args(args).stdin(crate::exec::Io::Null);
        for (k, v) in [("GIT_AUTHOR_NAME", "Launchpad"), ("GIT_AUTHOR_EMAIL", "launchpad@localhost"), ("GIT_COMMITTER_NAME", "Launchpad"), ("GIT_COMMITTER_EMAIL", "launchpad@localhost")] {
            cmd.env(k, v);
        }
        cmd.output().await
    };
    if !dir.join(".git").exists() {
        let _ = run(&["init", "--quiet"]).await;
    }
    let _ = run(&["add", "--all"]).await;
    let _ = run(&["commit", "--quiet", "--allow-empty", "-m", "Start of the session (Launchpad)"]).await;
}

/// Sends every change of the copy since the last sync to Home. Returns how many files changed.
pub async fn sync_up(app: &App, id: &str, dir: &Path, session: &str) -> Result<usize> {
    if !dir.is_dir() {
        return Ok(0);
    }
    let state = state_file(app, dir);
    let synced: Synced = tokio::fs::read(&state).await.ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let mut updates = vec![];
    let mut now: BTreeMap<String, String> = BTreeMap::new();
    for rel in walk(dir) {
        if check_path(&rel).is_err() {
            continue;
        }
        let (sha, size, executable) = hash_file(&dir.join(&rel)).await?;
        if synced.files.get(&rel) != Some(&sha) {
            home::upload(app, id, &sha, tokio::fs::read(dir.join(&rel)).await?).await?;
            updates.push(Update { path: rel.clone(), sha256: Some(sha.clone()), size, executable });
        }
        now.insert(rel, sha);
    }
    for rel in synced.files.keys().filter(|r| !now.contains_key(*r)) {
        updates.push(Update { path: rel.clone(), sha256: None, size: 0, executable: false });
    }
    if updates.is_empty() {
        return Ok(0);
    }
    let n = updates.len();
    let version = home::commit(app, id, session, updates).await?;
    tokio::fs::write(&state, serde_json::to_vec(&Synced { version, files: now })?).await?;
    Ok(n)
}

/// Releases the lock (after Archive).
pub async fn release(app: &App, id: &str, session: &str) -> Result<()> {
    home::lock(app, id, session, false).await
}

/// The asset folders a worktree maps in `.launchpad/assets.toml`: `path = "<asset id>"`.
pub async fn mapped_assets(root: &Path) -> Vec<(String, String)> {
    let Ok(text) = tokio::fs::read_to_string(root.join(".launchpad/assets.toml")).await else { return vec![] };
    let Ok(table) = toml::from_str::<toml::Table>(&text) else { return vec![] };
    table.into_iter().filter_map(|(k, v)| Some((k, v.as_str()?.to_string()))).filter(|(k, _)| check_path(k).is_ok()).collect()
}

/// The stores a session writes, with their folders on this machine: its plain folder, and the asset folders
/// of its worktrees.
pub async fn session_stores(app: &App, session_id: &str) -> Vec<(String, PathBuf)> {
    let mut out = vec![];
    if let Some(s) = app.sessions.get(session_id)
        && let Some(pid) = &s.project_id
        && let Ok(Some(p)) = crate::projects::get(app, pid).await
        && p.plain
    {
        out.push((p.id.clone(), PathBuf::from(&p.path)));
    }
    for w in crate::worktrees::for_session(app, session_id).await.unwrap_or_default().into_iter().filter(|w| w.state == crate::worktrees::WorktreeState::Active) {
        for (path, id) in mapped_assets(Path::new(&w.root)).await {
            out.push((id, Path::new(&w.root).join(path)));
        }
    }
    out
}

/// Before a session starts: its plain folder and asset folders come down from Home.
pub async fn prepare(app: &App, session_id: &str) -> Result<()> {
    for (id, dir) in session_stores(app, session_id).await {
        sync_down(app, &id, &dir, session_id).await?;
        exclude_asset(app, &dir).await;
    }
    Ok(())
}

/// Asset folders stay out of Git: they are listed in the clone's `info/exclude`.
async fn exclude_asset(app: &App, dir: &Path) {
    let Some(w) = crate::worktrees::find_by_path(app, &dir.to_string_lossy()).await.ok().flatten() else { return };
    let Ok(rel) = dir.strip_prefix(&w.root) else { return };
    let line = format!("/{}/", rel.to_string_lossy().replace('\\', "/"));
    let file = Path::new(&w.repo).join(".git/info/exclude");
    let text = tokio::fs::read_to_string(&file).await.unwrap_or_default();
    if !text.lines().any(|l| l == line) {
        let _ = tokio::fs::create_dir_all(file.parent().unwrap()).await;
        let _ = tokio::fs::write(&file, format!("{text}{}{line}\n", if text.is_empty() || text.ends_with('\n') { "" } else { "\n" })).await;
    }
}

/// At the end of every turn: changes go up to Home.
pub async fn after_turn(app: &App, session_id: &str) {
    for (id, dir) in session_stores(app, session_id).await {
        match sync_up(app, &id, &dir, session_id).await {
            Ok(0) => {}
            Ok(n) => tracing::debug!("{session_id}: {n} files of {} saved on Home", dir.display()),
            Err(e) => app.sessions.sink(app, session_id).emit(crate::model::EventBody::Notice { text: format!("Could not save {} on Home: {e:#}. Launchpad retries at the end of the next turn.", dir.display()) }).await,
        }
    }
}

/// After Archive: a last sync, then the lock goes back to Home.
pub async fn after_archive(app: &App, session_id: &str) {
    let stores = session_stores(app, session_id).await;
    after_turn(app, session_id).await;
    for (id, _) in stores {
        if let Err(e) = release(app, &id, session_id).await {
            tracing::warn!("release {id}: {e:#}");
        }
    }
}

/// `launchpad assets create <path>`: a new asset folder at `path` of the session's worktree, mapped in
/// `.launchpad/assets.toml` (commit that file). Existing files in it are uploaded.
pub async fn create_in_worktree(app: &App, session_id: &str, cwd: &str, path: &str) -> Result<String> {
    let w = crate::worktrees::find_by_path(app, cwd).await?.context("run this in a Launchpad worktree")?;
    ensure!(w.sessions.iter().any(|s| s == session_id), "this worktree belongs to another session");
    let path = path.trim().trim_matches('/').to_string();
    check_path(&path)?;
    ensure!(!mapped_assets(Path::new(&w.root)).await.iter().any(|(p, _)| *p == path), "{path} is already an asset folder");
    let name = format!("{}/{path}", w.branch);
    let asset: Asset = if crate::home_git::here(app) {
        create_asset(app, &w.project_id, &name).await?
    } else {
        serde_json::from_value(crate::home_link::home_request("POST", &format!("/api/plain/assets/{}", w.project_id), Some(serde_json::json!({"name": name}))).await?)?
    };
    let manifest = Path::new(&w.root).join(".launchpad/assets.toml");
    tokio::fs::create_dir_all(manifest.parent().unwrap()).await?;
    let mut text = tokio::fs::read_to_string(&manifest).await.unwrap_or_else(|_| "# Asset folders: stored by Launchpad, synced into worktrees, not in Git.\n".into());
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&format!("{} = \"{}\"\n", toml::Value::String(path.clone()), asset.id));
    tokio::fs::write(&manifest, text).await?;
    let dir = Path::new(&w.root).join(&path);
    tokio::fs::create_dir_all(&dir).await?;
    home::lock(app, &asset.id, session_id, true).await?;
    exclude_asset(app, &dir).await;
    sync_up(app, &asset.id, &dir, session_id).await?;
    Ok(format!("Asset folder `{path}` created (id {}). Commit `.launchpad/assets.toml`; files you put in `{path}` are stored by Launchpad (not in Git) at the end of every turn, with previous versions kept 30 days.", asset.id))
}

//! Home's backups: a consistent copy of `state.db` (and `config.toml`) every minute when something changed,
//! and before a planned restart. An empty data folder restores the newest complete backup on boot.
//!
//! Layout under the destination (an HF bucket or a local folder):
//! - `home/blobs/sha256/<hash>`: content-addressed objects, never overwritten.
//! - `home/backup/<time>/manifest.json`: the objects of one backup, by hash. Written last: a backup is complete
//!   only when its manifest exists, so restore never points to objects that were never uploaded.
//!
//! Credentials: with `LAUNCHPAD_HOME_KEY` (base64 of 32 random bytes; the hosted Home keeps it in a Space
//! secret), the vault's secrets are sealed with AES-256-GCM inside the backup copy of `state.db`, and opened again
//! on restore. Everything else stays plain, so the bucket can be browsed and deduplicated.
//!
//! Other files a restore needs go in as objects too: uploads (`uploads/<sha256>`) and `runner-archives.json`
//! (`data_files`). Plain folder blobs and handoff files are listed as `plain/<sha256>` and fetched on first use.
//!
//! Retention (`prune`, after each complete backup, one at a time with backups): keep the newest backup, the one
//! before it, the newest one at least an hour old, and the newest one at least 1, 2, …, 7 days old (`retained`);
//! delete the other manifests, then the objects no remaining manifest lists (never one uploaded in the last hour).
//! `cache.db` is not backed up: it is rebuilt from GitHub.
//!
//! Home Git: each mirror adds a bundle (`git/<folder-id>.bundle`) of its Launchpad branches and kept tips, minus
//! what the folder's remote has (a full bundle without a remote). After a restore, a mirror is cloned from the
//! remote again on first use, then gets its bundle (`git_restore.json` lists them).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ts_rs::TS;

use crate::App;
use crate::shellenv::UserEnv;
use crate::util::now_ms;

const HOUR_MS: i64 = 3600 * 1000;
const DAY_MS: i64 = 24 * HOUR_MS;

/// Where backups go: `hf://buckets/<namespace>/<bucket>` or a local folder.
#[derive(Clone, Debug, PartialEq)]
pub enum Store {
    Bucket(String),
    Dir(PathBuf),
}

impl Store {
    pub fn parse(s: &str) -> Option<Store> {
        let s = s.trim().trim_end_matches('/');
        if s.is_empty() {
            None
        } else if let Some(rest) = s.strip_prefix("hf://buckets/") {
            Some(Store::Bucket(rest.into()))
        } else {
            Some(Store::Dir(crate::paths::expand_home(s)))
        }
    }

    pub(crate) async fn put(&self, env: &UserEnv, key: &str, data: Vec<u8>) -> Result<()> {
        match self {
            Store::Bucket(b) => crate::hf::hf(env, &["buckets", "cp", "-", &format!("hf://buckets/{b}/{key}"), "--quiet"], Some(data)).await.map(|_| ()),
            Store::Dir(d) => {
                let path = d.join(key);
                tokio::fs::create_dir_all(path.parent().context("bad key")?).await?;
                // Write then rename, so a crash never leaves a partial object under its final name.
                let tmp = path.with_extension("partial");
                tokio::fs::write(&tmp, data).await?;
                tokio::fs::rename(&tmp, &path).await?;
                Ok(())
            }
        }
    }

    async fn get(&self, env: &UserEnv, key: &str) -> Result<Vec<u8>> {
        match self {
            Store::Bucket(b) => {
                let mut cmd = crate::procs::command(env, "hf", None);
                cmd.args(["buckets", "cp", &format!("hf://buckets/{b}/{key}"), "-"]).stdin(crate::exec::Io::Null).stderr(crate::exec::Io::Null);
                let out = tokio::time::timeout(Duration::from_secs(600), cmd.output()).await.context("hf timed out")??;
                anyhow::ensure!(out.status.success(), "could not download {key}");
                Ok(out.stdout)
            }
            Store::Dir(d) => Ok(tokio::fs::read(d.join(key)).await.with_context(|| format!("read {key}"))?),
        }
    }

    /// Keys of the files under `prefix` (recursive).
    async fn list(&self, env: &UserEnv, prefix: &str) -> Result<Vec<String>> {
        match self {
            Store::Bucket(b) => {
                let out = match crate::hf::hf(env, &["buckets", "ls", &format!("{b}/{prefix}"), "-R", "--format", "json"], None).await {
                    Ok(out) => out,
                    // A prefix that does not exist yet.
                    Err(_) => return Ok(vec![]),
                };
                let items: Vec<serde_json::Value> = serde_json::from_str(out.trim()).unwrap_or_default();
                Ok(items.iter().filter(|i| i["type"] == "file").filter_map(|i| i["path"].as_str()).map(|p| if p.starts_with(prefix) { p.to_string() } else { format!("{prefix}/{p}") }).collect())
            }
            Store::Dir(d) => {
                let root = d.join(prefix);
                let mut out = vec![];
                let mut stack = vec![root];
                while let Some(dir) = stack.pop() {
                    let Ok(mut rd) = tokio::fs::read_dir(&dir).await else { continue };
                    while let Some(e) = rd.next_entry().await? {
                        let path = e.path();
                        if e.file_type().await?.is_dir() {
                            stack.push(path);
                        } else if path.extension().is_none_or(|x| x != "partial")
                            && let Ok(rel) = path.strip_prefix(d)
                        {
                            out.push(rel.to_string_lossy().replace('\\', "/"));
                        }
                    }
                }
                Ok(out)
            }
        }
    }

    pub(crate) async fn remove(&self, env: &UserEnv, key: &str) -> Result<()> {
        match self {
            Store::Bucket(b) => crate::hf::hf(env, &["buckets", "rm", &format!("{b}/{key}"), "--quiet"], None).await.map(|_| ()),
            Store::Dir(d) => Ok(tokio::fs::remove_file(d.join(key)).await?),
        }
    }

    /// The files under `prefix` (recursive) with their size and upload time. Unlike `list`, a failed listing is an
    /// error, never an empty list: pruning decides what to delete from it.
    async fn entries(&self, env: &UserEnv, prefix: &str) -> Result<Vec<Entry>> {
        match self {
            Store::Bucket(b) => {
                let out = crate::hf::hf(env, &["buckets", "ls", &format!("{b}/{prefix}"), "-R", "--format", "json"], None).await?;
                let out = out.trim();
                // hf prints this instead of JSON for an empty listing.
                if out.is_empty() || out == "(empty)" {
                    return Ok(vec![]);
                }
                let items: Vec<serde_json::Value> = serde_json::from_str(out).context("unexpected hf buckets ls output")?;
                let time = |v: &serde_json::Value| v.as_str().and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()).map(|t| t.timestamp_millis());
                Ok(items
                    .iter()
                    .filter(|i| i["type"] == "file")
                    .filter_map(|i| {
                        let p = i["path"].as_str()?;
                        let key = if p.starts_with(prefix) { p.to_string() } else { format!("{prefix}/{p}") };
                        Some(Entry { key, size: i["size"].as_u64(), uploaded_at: time(&i["uploaded_at"]).or_else(|| time(&i["mtime"])) })
                    })
                    .collect())
            }
            Store::Dir(d) => {
                let mut out = vec![];
                let mut stack = vec![d.join(prefix)];
                while let Some(dir) = stack.pop() {
                    let mut rd = match tokio::fs::read_dir(&dir).await {
                        Ok(rd) => rd,
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(e) => return Err(anyhow::Error::from(e).context(format!("list {}", dir.display()))),
                    };
                    while let Some(e) = rd.next_entry().await? {
                        let path = e.path();
                        let meta = e.metadata().await?;
                        if meta.is_dir() {
                            stack.push(path);
                        } else if path.extension().is_none_or(|x| x != "partial")
                            && let Ok(rel) = path.strip_prefix(d)
                        {
                            let uploaded_at = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|t| t.as_millis() as i64);
                            out.push(Entry { key: rel.to_string_lossy().replace('\\', "/"), size: Some(meta.len()), uploaded_at });
                        }
                    }
                }
                Ok(out)
            }
        }
    }

    /// Deletes these files, all under `prefix`: a bucket in batches (one listing and one batch call each), a folder
    /// a few files at a time.
    async fn remove_many(&self, env: &UserEnv, prefix: &str, keys: &[String]) -> Result<()> {
        for k in keys {
            // Exact names only: `--include` takes glob patterns.
            ensure!(k.starts_with(&format!("{prefix}/")) && !k.contains(['*', '?', '[', ']', '\\']) && !k.split('/').any(|s| s.is_empty() || s == ".." || s == "."), "refusing to delete {k}");
        }
        match self {
            Store::Bucket(b) => {
                for chunk in keys.chunks(100) {
                    let target = format!("{b}/{prefix}");
                    let mut args: Vec<&str> = vec!["buckets", "rm", &target, "--recursive", "--yes", "--quiet"];
                    for k in chunk {
                        args.extend(["--include", k.as_str()]);
                    }
                    crate::hf::hf(env, &args, None).await.with_context(|| format!("delete {} files under {prefix}", chunk.len()))?;
                }
                Ok(())
            }
            Store::Dir(d) => {
                // A few at a time: a bucket mounted as a folder makes each delete a network call.
                for chunk in keys.chunks(8) {
                    futures_util::future::try_join_all(chunk.iter().map(|k| remove_in(d, prefix, k))).await?;
                }
                Ok(())
            }
        }
    }
}

/// Deletes a file of a folder destination, and its backup's folder (`<time>/manifest.json`) once empty.
async fn remove_in(d: &Path, prefix: &str, key: &str) -> Result<()> {
    let path = d.join(key);
    match tokio::fs::remove_file(&path).await {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(anyhow::Error::from(e).context(format!("delete {key}"))),
        _ => {}
    }
    if let Some(parent) = path.parent().filter(|p| *p != d.join(prefix)) {
        let _ = tokio::fs::remove_dir(parent).await;
    }
    Ok(())
}

/// A file of the destination, from a listing.
#[derive(Clone, Debug)]
struct Entry {
    key: String,
    size: Option<u64>,
    /// Milliseconds; None when the listing does not say.
    uploaded_at: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Object {
    /// `state.db` or `config.toml`.
    pub name: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub created_at: i64,
    pub commit: String,
    pub objects: Vec<Object>,
}

/// Shown in Settings → Home and runners.
#[derive(Clone, Debug, Default, Serialize, TS)]
pub struct BackupInfo {
    pub destination: Option<String>,
    #[ts(type = "number | null")]
    pub last_complete_at: Option<i64>,
    pub error: Option<String>,
    /// Vault secrets in backups are sealed with `LAUNCHPAD_HOME_KEY`.
    pub credentials_encrypted: bool,
    /// Why the last pass could not write every trace copy (`trace_copies`).
    pub traces_error: Option<String>,
    /// The last pruning of old backups (`prune`): when it completed and what it did.
    #[ts(type = "number | null")]
    pub pruned_at: Option<i64>,
    pub prune: Option<String>,
    /// Why the last pruning was skipped or stopped.
    pub prune_error: Option<String>,
}

#[derive(Clone, Default)]
pub struct Backups {
    info: Arc<Mutex<BackupInfo>>,
    /// `total_changes()` of the state database at the last backup.
    changes: Arc<Mutex<Option<i64>>>,
    running: Arc<tokio::sync::Mutex<()>>,
    /// Per mirror: the hash of its refs and the bundle object saved for them.
    bundles: Arc<Mutex<std::collections::HashMap<String, (String, Object)>>>,
    /// A pruning pass waits for `running`.
    prune_queued: Arc<std::sync::atomic::AtomicBool>,
    /// Manifests read by pruning, by key (they never change).
    manifests: Arc<Mutex<std::collections::HashMap<String, Vec<u8>>>>,
}

static GIT_CHANGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Home Git or a file of `data_files` changed: the next backup runs even if the database did not.
pub fn changed() {
    GIT_CHANGED.store(true, std::sync::atomic::Ordering::Relaxed);
}

impl Backups {
    pub fn info(&self) -> BackupInfo {
        BackupInfo { credentials_encrypted: sealing_key().is_some(), ..self.info.lock().unwrap().clone() }
    }
}

const MAGIC: &[u8] = b"LPSEALED1";
/// Where backups go.
const ROOT: &str = "home";
/// A sealed vault secret in a backup copy of `state.db`.
const SEALED_SECRET: &str = "lpsealed1:";

static KEY: std::sync::OnceLock<Option<[u8; 32]>> = std::sync::OnceLock::new();

/// The backup encryption key from `LAUNCHPAD_HOME_KEY`, if set.
fn sealing_key() -> Option<&'static [u8; 32]> {
    KEY.get_or_init(|| {
        let value = std::env::var("LAUNCHPAD_HOME_KEY").ok().filter(|v| !v.trim().is_empty())?;
        // A Home with a broken key must never fall back to plain-text backups.
        Some(parse_key(&value).expect("LAUNCHPAD_HOME_KEY must be base64 of 32 bytes"))
    })
    .as_ref()
}

fn parse_key(value: &str) -> Option<[u8; 32]> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(value.trim()).ok()?.try_into().ok()
}

/// `MAGIC || nonce (12) || AES-256-GCM ciphertext and tag`.
fn seal(key: &[u8; 32], data: &[u8]) -> Result<Vec<u8>> {
    use aes_gcm::{Aes256Gcm, KeyInit, aead::Aead};
    let nonce: [u8; 12] = rand::random();
    let sealed = Aes256Gcm::new(key.into()).encrypt(&nonce.into(), data).map_err(|_| anyhow::anyhow!("encryption failed"))?;
    Ok([MAGIC, &nonce, &sealed].concat())
}

/// Opens a sealed value.
fn open(key: &[u8; 32], data: Vec<u8>) -> Result<Vec<u8>> {
    use aes_gcm::{Aes256Gcm, KeyInit, aead::Aead};
    let rest = data.strip_prefix(MAGIC).context("not an encrypted value")?;
    ensure!(rest.len() >= 12 + 16, "truncated encrypted object");
    let (nonce, sealed) = rest.split_at(12);
    let nonce: [u8; 12] = nonce.try_into()?;
    Aes256Gcm::new(key.into()).decrypt(&nonce.into(), sealed).map_err(|_| anyhow::anyhow!("wrong key or damaged object"))
}

fn blob_key(hash: &str) -> String {
    format!("{ROOT}/blobs/sha256/{hash}")
}

fn manifest_key(at: i64) -> String {
    let t = chrono::DateTime::from_timestamp_millis(at).unwrap_or_default().format("%Y%m%dT%H%M%S%.3fZ");
    format!("{ROOT}/backup/{t}/manifest.json")
}

/// An object by hash.
async fn get_blob(store: &Store, env: &UserEnv, hash: &str) -> Result<Vec<u8>> {
    let data = store.get(env, &blob_key(hash)).await?;
    ensure!(hex::encode(Sha256::digest(&data)) == hash, "backup object {hash} does not match its hash");
    Ok(data)
}

/// Complete backups under `root`, newest first.
async fn manifests(store: &Store, env: &UserEnv, root: &str) -> Result<Vec<(String, Manifest)>> {
    let mut out = vec![];
    for key in store.list(env, &format!("{root}/backup")).await?.into_iter().filter(|k| k.ends_with("/manifest.json")) {
        match store.get(env, &key).await.and_then(|b| Ok(serde_json::from_slice::<Manifest>(&b)?)) {
            Ok(m) => out.push((key, m)),
            Err(e) => tracing::warn!("home backup: unreadable {key}: {e:#}"),
        }
    }
    out.sort_by_key(|(_, m)| std::cmp::Reverse(m.created_at));
    Ok(out)
}


async fn snapshot(app: &App) -> Result<(Vec<u8>, i64)> {
    let tmp = app.paths.data_dir.join(format!("state.backup-{}.db", uuid::Uuid::new_v4()));
    let target = tmp.clone();
    let changes = app
        .state_db
        .call(move |c| {
            // One consistent copy, taken by SQLite itself while the database stays open.
            c.execute("VACUUM INTO ?1", [target.to_string_lossy().as_ref()])?;
            Ok(c.query_row("SELECT total_changes()", [], |r| r.get::<_, i64>(0))?)
        })
        .await?;
    let sealed = match sealing_key() {
        Some(key) => {
            let (path, key) = (tmp.clone(), *key);
            tokio::task::spawn_blocking(move || map_secrets(&path, |s| seal_secret(&key, s))).await?
        }
        None => Ok(()),
    };
    let bytes = match sealed {
        Ok(()) => tokio::fs::read(&tmp).await.map_err(anyhow::Error::from),
        Err(e) => Err(e.context("seal the vault in the backup")),
    };
    let _ = tokio::fs::remove_file(&tmp).await;
    Ok((bytes?, changes))
}

fn seal_secret(key: &[u8; 32], secret: &str) -> Result<String> {
    use base64::Engine;
    Ok(format!("{SEALED_SECRET}{}", base64::engine::general_purpose::STANDARD.encode(seal(key, secret.as_bytes())?)))
}

fn open_secret(key: Option<&[u8; 32]>, value: &str) -> Result<String> {
    use base64::Engine;
    let Some(rest) = value.strip_prefix(SEALED_SECRET) else { return Ok(value.to_string()) };
    let key = key.context("this backup's vault is encrypted: set LAUNCHPAD_HOME_KEY (the LP_HOME_KEY Space secret)")?;
    let data = open(key, base64::engine::general_purpose::STANDARD.decode(rest)?)?;
    Ok(String::from_utf8(data)?)
}

/// Rewrites every vault secret of a database copy (backup or restore); the only credentials in `state.db`.
fn map_secrets(db: &Path, f: impl Fn(&str) -> Result<String>) -> Result<()> {
    let mut c = rusqlite::Connection::open(db)?;
    let exists: bool = c.query_row("SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'vault')", [], |r| r.get(0))?;
    if !exists {
        return Ok(());
    }
    let tx = c.transaction()?;
    let rows: Vec<(String, String)> = tx.prepare("SELECT id, secret FROM vault")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    for (id, secret) in rows {
        tx.execute("UPDATE vault SET secret = ?1 WHERE id = ?2", rusqlite::params![f(&secret)?, id])?;
    }
    tx.commit()?;
    // VACUUM drops the old pages, so no plain secret survives in the free space of the copy.
    c.execute_batch("VACUUM")?;
    Ok(())
}

/// Takes one backup now if the database changed since the last one (or always with `force`).
pub async fn run(app: &App, force: bool) -> Result<bool> {
    let Some(store) = Store::parse(&app.config.get().home.backup) else { return Ok(false) };
    let backups = app.home_backups.clone();
    let _one = backups.running.lock().await;
    let current = app.state_db.call(|c| Ok(c.query_row("SELECT total_changes()", [], |r| r.get::<_, i64>(0))?)).await?;
    let git_changed = GIT_CHANGED.swap(false, std::sync::atomic::Ordering::Relaxed);
    if !force && !git_changed && *backups.changes.lock().unwrap() == Some(current) {
        return Ok(false);
    }
    let (db, changes) = snapshot(app).await?;
    let config = tokio::fs::read(app.paths.config_file()).await.unwrap_or_default();
    let existing: std::collections::HashSet<String> = store.list(&app.env, &format!("{ROOT}/blobs/sha256")).await?.into_iter().collect();
    let mut objects = vec![];
    for (name, data) in [("state.db", db), ("config.toml", config)] {
        let hash = hex::encode(Sha256::digest(&data));
        let size = data.len() as u64;
        if !existing.contains(&blob_key(&hash)) {
            store.put(&app.env, &blob_key(&hash), data).await?;
        }
        objects.push(Object { name: name.into(), sha256: hash, size });
    }
    match bundles(app, &store, &existing).await {
        Ok(mut b) => objects.append(&mut b),
        Err(e) => {
            changed();
            return Err(e.context("bundle Home Git"));
        }
    }
    // Plain and asset folders: their contents are already content-addressed.
    for sha in crate::plain::live_blobs(app).await? {
        if !existing.contains(&blob_key(&sha)) {
            let data = match tokio::fs::read(crate::plain::blob_path(app, &sha)?).await {
                Ok(d) => d,
                // Not fetched since a restore: carry it over from the backup it came from.
                Err(_) => match get_blob(&store, &app.env, &sha).await {
                    Ok(d) => d,
                    Err(_) => continue,
                },
            };
            store.put(&app.env, &blob_key(&sha), data).await?;
        }
        objects.push(Object { name: format!("plain/{sha}"), sha256: sha, size: 0 });
    }
    for (name, path) in data_files(&app.paths).await {
        let Ok(data) = tokio::fs::read(&path).await else { continue };
        let hash = hex::encode(Sha256::digest(&data));
        let size = data.len() as u64;
        if !existing.contains(&blob_key(&hash)) {
            store.put(&app.env, &blob_key(&hash), data).await?;
        }
        objects.push(Object { name, sha256: hash, size });
    }
    let now = now_ms();
    let manifest = Manifest { format: 1, created_at: now, commit: crate::commit().into(), objects };
    store.put(&app.env, &manifest_key(now), serde_json::to_vec_pretty(&manifest)?).await?;
    *backups.changes.lock().unwrap() = Some(changes);
    backups.info.lock().unwrap().last_complete_at = Some(now);
    spawn_prune(app, store.clone(), manifest_key(now));
    if let Err(e) = readable(app, &store).await {
        tracing::warn!("home backup: readable copies: {e:#}");
    }
    let traces = crate::trace_copies::run(app, &store).await;
    if let Err(e) = &traces {
        tracing::warn!("home backup: trace copies: {e:#}");
    }
    backups.info.lock().unwrap().traces_error = traces.err().map(|e| format!("{e:#}"));
    Ok(true)
}

/// Home's files outside `state.db` that a restore needs, by object name: uploads (`uploads/<sha256>`: the
/// attachments of drafts, scheduled launches and routines) and what waits for runners that are away
/// (`runner-archives.json`).
async fn data_files(paths: &crate::paths::Paths) -> Vec<(String, PathBuf)> {
    let mut out = vec![];
    let archives = paths.data_dir.join(ARCHIVES);
    if archives.is_file() {
        out.push((ARCHIVES.to_string(), archives));
    }
    if let Ok(mut rd) = tokio::fs::read_dir(paths.data_dir.join("uploads")).await {
        while let Ok(Some(e)) = rd.next_entry().await {
            let name = e.file_name().to_string_lossy().into_owned();
            if is_hash(&name) {
                out.push((format!("uploads/{name}"), e.path()));
            }
        }
    }
    out
}

/// `runner-archives.json` (`runners::archives_file`).
const ARCHIVES: &str = "runner-archives.json";

fn is_hash(name: &str) -> bool {
    name.len() == 64 && name.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

/// Where a restore writes a backup object; None for objects restored otherwise (Home Git bundles, plain blobs
/// fetched on first use).
fn restore_path(paths: &crate::paths::Paths, name: &str) -> Option<PathBuf> {
    match name {
        "state.db" => Some(paths.state_db()),
        // Settings edited since (or set by the Space) win over the backup.
        "config.toml" if !paths.config_file().exists() => Some(paths.config_file()),
        ARCHIVES => Some(paths.data_dir.join(ARCHIVES)),
        _ => name.strip_prefix("uploads/").filter(|h| is_hash(h)).map(|h| paths.data_dir.join("uploads").join(h)),
    }
}

/// Top-level names the bucket already uses (Home backups, the retired Space connector, older trace exports).
const RESERVED: &[&str] = &[ROOT, "space", "projects", "traces", "backups", "credentials"];

/// The bucket prefix of a workspace: its name, like its knowledge base folder on runners.
pub fn workspace_prefix(name: &str, id: &str) -> String {
    let p = crate::kb::folder_name(name, id);
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(&p)) { format!("{p} {}", &id[..8.min(id.len())]) } else { p }
}

/// Readable copies next to the backups, for browsing the bucket: `<workspace>/kb/<note>` and
/// `<workspace>/bots/<slug>/<file>`. Only changed files are uploaded and removed files are deleted. Home never reads
/// them back: its own store (in every backup) stays the source.
async fn readable(app: &App, store: &Store) -> Result<()> {
    let state = app.paths.data_dir.join("readable-copies.json");
    let mut done: std::collections::BTreeMap<String, String> = tokio::fs::read(&state).await.ok().and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or_default();
    let mut want = std::collections::BTreeMap::new();
    let workspaces = crate::workspace::list(app).await?;
    for w in &workspaces {
        let prefix = workspace_prefix(&w.name, &w.id);
        for f in crate::plain::tree(app, &crate::kb::store(&w.id)).await.map(|t| t.files).unwrap_or_default() {
            want.insert(format!("{prefix}/kb/{}", f.path), f.sha256);
        }
    }
    for b in crate::bots::list(app).await.unwrap_or_default() {
        let Some(w) = workspaces.iter().find(|w| w.id == b.workspace_id) else { continue };
        for f in crate::plain::tree(app, &crate::bots::store(&b.id)).await.map(|t| t.files).unwrap_or_default() {
            want.insert(format!("{}/bots/{}/{}", workspace_prefix(&w.name, &w.id), b.slug, f.path), f.sha256);
        }
    }
    let result: Result<()> = async {
        for (key, sha) in &want {
            if done.get(key) != Some(sha) {
                store.put(&app.env, key, crate::plain::read_blob(app, sha).await?).await?;
                done.insert(key.clone(), sha.clone());
            }
        }
        let gone: Vec<String> = done.keys().filter(|k| !want.contains_key(*k)).cloned().collect();
        for key in gone {
            store.remove(&app.env, &key).await?;
            done.remove(&key);
        }
        Ok(())
    }.await;
    write_atomic(&state, &serde_json::to_vec(&done)?)?;
    result
}

/// One bundle per mirror: Launchpad branches, kept tips and leftovers the remote does not have.
async fn bundles(app: &App, store: &Store, existing: &std::collections::HashSet<String>) -> Result<Vec<Object>> {
    let mut out = vec![];
    // A fresh restore has pending bundles but no mirror directory yet.
    tokio::fs::create_dir_all(crate::home_git::root(app)).await?;
    let mut entries = tokio::fs::read_dir(crate::home_git::root(app)).await?;
    while let Some(e) = entries.next_entry().await? {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_suffix(".git").filter(|n| !n.starts_with('.')) else { continue };
        let dir = e.path();
        let refs = crate::home_git::git_in(app, &dir, &["for-each-ref", "--format=%(objectname) %(refname)"], 60).await?;
        let refs_hash = hex::encode(Sha256::digest(refs.as_bytes()));
        if let Some((h, o)) = app.home_backups.bundles.lock().unwrap().get(id).cloned()
            && h == refs_hash
            && existing.contains(&blob_key(&o.sha256))
        {
            out.push(o);
            continue;
        }
        let ours: Vec<&str> = refs.lines().filter_map(|l| l.split_once(' ').map(|(_, r)| r)).filter(|r| r.starts_with("refs/heads/lp") || r.starts_with("refs/kept/") || r.starts_with("refs/lp-snapshots/")).collect();
        let theirs: Vec<String> = refs.lines().filter_map(|l| l.split_once(' ')).filter(|(_, r)| !(r.starts_with("refs/heads/lp") || r.starts_with("refs/kept/") || r.starts_with("refs/lp-snapshots/"))).map(|(c, _)| format!("^{c}")).collect();
        let has_remote = crate::home_git::git_in(app, &dir, &["remote"], 10).await?.lines().any(|r| r == "origin");
        let file = app.paths.data_dir.join(format!("bundle-{}.partial", uuid::Uuid::new_v4()));
        let file_s = file.to_string_lossy().into_owned();
        let mut args: Vec<String> = vec!["bundle".into(), "create".into(), "--quiet".into(), file_s.clone()];
        if has_remote {
            if ours.is_empty() {
                app.home_backups.bundles.lock().unwrap().remove(id);
                continue;
            }
            args.extend(ours.iter().map(|r| r.to_string()));
            args.extend(theirs);
        } else {
            args.push("--all".into());
        }
        let refs_args: Vec<&str> = args.iter().map(String::as_str).collect();
        // Nothing new beyond the remote: Git refuses an empty bundle.
        if let Err(e) = crate::home_git::git_in(app, &dir, &refs_args, 600).await {
            let _ = tokio::fs::remove_file(&file).await;
            if format!("{e:#}").contains("empty bundle") { continue }
            return Err(e);
        }
        let data = tokio::fs::read(&file).await;
        let _ = tokio::fs::remove_file(&file).await;
        let data = data?;
        let hash = hex::encode(Sha256::digest(&data));
        let size = data.len() as u64;
        if !existing.contains(&blob_key(&hash)) {
            store.put(&app.env, &blob_key(&hash), data).await?;
        }
        let o = Object { name: format!("git/{id}.bundle"), sha256: hash, size };
        app.home_backups.bundles.lock().unwrap().insert(id.to_string(), (refs_hash, o.clone()));
        out.push(o);
    }
    // A restored mirror may not have been opened yet. Carry its bundle forward so pruning cannot
    // delete the only copy of its local branches while Home is idle.
    if let Ok(data) = tokio::fs::read(git_restore_file(&app.paths)).await {
        let pending: std::collections::BTreeMap<String, String> = serde_json::from_slice(&data)?;
        for (id, hash) in pending {
            let name = format!("git/{id}.bundle");
            if !out.iter().any(|o| o.name == name) {
                if !existing.contains(&blob_key(&hash)) {
                    let data = get_blob(store, &app.env, &hash).await?;
                    store.put(&app.env, &blob_key(&hash), data).await?;
                }
                out.push(Object { name, sha256: hash, size: 0 });
            }
        }
    }
    Ok(out)
}

/// A restored Home reads plain folder contents it no longer has from the backup destination.
pub async fn fetch_blob(app: &App, sha: &str) -> Result<Vec<u8>> {
    let store = Store::parse(&app.config.get().home.backup).context("no backup destination")?;
    get_blob(&store, &app.env, sha).await
}

fn git_restore_file(paths: &crate::paths::Paths) -> PathBuf {
    paths.data_dir.join("git_restore.json")
}

/// After a restore: applies a mirror's bundle once the mirror exists again (cloned from the remote).
pub async fn restore_bundle(app: &App, project_id: &str) -> Result<()> {
    let file = git_restore_file(&app.paths);
    let Ok(text) = tokio::fs::read_to_string(&file).await else { return Ok(()) };
    let mut pending: std::collections::BTreeMap<String, String> = serde_json::from_str(&text).unwrap_or_default();
    let Some(hash) = pending.get(project_id).cloned() else { return Ok(()) };
    let store = Store::parse(&app.config.get().home.backup).context("no backup destination to restore Home Git from")?;
    let data = get_blob(&store, &app.env, &hash).await.with_context(|| format!("the Home Git bundle of {project_id}"))?;
    let path = app.paths.data_dir.join(format!("restore-{project_id}.bundle"));
    tokio::fs::write(&path, data).await?;
    let dir = crate::home_git::mirror(app, project_id);
    let result = crate::home_git::git_in(app, &dir, &["fetch", "--quiet", &path.to_string_lossy(), "+refs/*:refs/*"], 600).await;
    let _ = tokio::fs::remove_file(&path).await;
    result.context("apply the Home Git bundle (the remote may have lost commits Launchpad branches build on)")?;
    pending.remove(project_id);
    tokio::fs::write(&file, serde_json::to_vec(&pending)?).await?;
    Ok(())
}

/// The time of a backup from its manifest key (`manifest_key`).
fn key_time(key: &str) -> Option<i64> {
    let t = key.strip_prefix(&format!("{ROOT}/backup/"))?.strip_suffix("/manifest.json")?;
    chrono::NaiveDateTime::parse_from_str(t, "%Y%m%dT%H%M%S%.3fZ").ok().map(|t| t.and_utc().timestamp_millis())
}

/// The backups retention keeps, as indexes into `times` (each backup's time, None when unknown): the newest, the one
/// before it, the newest at least an hour old, and the newest at least 1, 2, …, 7 days old. A backup without a known
/// time, or with a time after `now` (a clock that went back), is kept too: pruning never deletes what it cannot place.
fn retained(times: &[Option<i64>], now: i64) -> std::collections::BTreeSet<usize> {
    let mut keep = std::collections::BTreeSet::new();
    let mut known: Vec<(i64, usize)> = vec![];
    for (i, t) in times.iter().enumerate() {
        match t {
            Some(t) if *t <= now => known.push((*t, i)),
            Some(t) => {
                known.push((*t, i));
                keep.insert(i);
            }
            None => {
                keep.insert(i);
            }
        }
    }
    // Newest first, whatever the clock says.
    known.sort_by(|a, b| b.cmp(a));
    keep.extend(known.iter().take(2).map(|(_, i)| *i));
    let ages = std::iter::once(HOUR_MS).chain((1..=7).map(|d| d * DAY_MS));
    for age in ages {
        if let Some((_, i)) = known.iter().find(|(t, _)| now - t >= age) {
            keep.insert(*i);
        }
    }
    keep
}

/// The objects a manifest needs, by key. One that does not parse keeps every hash it mentions.
fn references(data: &[u8]) -> Vec<String> {
    if let Ok(m) = serde_json::from_slice::<Manifest>(data) {
        return m.objects.iter().map(|o| blob_key(&o.sha256)).collect();
    }
    data.split(|b| !b.is_ascii_hexdigit()).filter(|w| w.len() == 64).map(|w| blob_key(&String::from_utf8_lossy(w).to_ascii_lowercase())).collect()
}

async fn manifest_keys(store: &Store, env: &UserEnv, dir: &str) -> Result<Vec<String>> {
    let mut keys: Vec<String> = store.entries(env, dir).await?.into_iter().map(|e| e.key).filter(|k| k.ends_with("/manifest.json")).collect();
    keys.sort();
    Ok(keys)
}

/// What one pruning pass did.
#[derive(Debug, Default)]
struct Pruned {
    kept: Vec<String>,
    manifests: usize,
    blobs: usize,
    bytes: u64,
}

/// A manifest's bytes. Manifests never change once written, so `cache` keeps them between passes.
async fn read_manifest(store: &Store, env: &UserEnv, key: &str, cache: &mut std::collections::HashMap<String, Vec<u8>>) -> Result<Vec<u8>> {
    if let Some(data) = cache.get(key) {
        return Ok(data.clone());
    }
    let data = store.get(env, key).await.with_context(|| format!("read {key}"))?;
    cache.insert(key.to_string(), data.clone());
    Ok(data)
}

/// A backup restore can use: its manifest parses and every object it lists is in `blobs`, with the recorded size
/// (objects recorded with size 0, plain blobs and carried-over bundles, only need to exist).
async fn check_complete(store: &Store, env: &UserEnv, key: &str, blobs: &std::collections::HashMap<String, Entry>, cache: &mut std::collections::HashMap<String, Vec<u8>>) -> Result<()> {
    let m: Manifest = serde_json::from_slice(&read_manifest(store, env, key, cache).await?).with_context(|| format!("parse {key}"))?;
    ensure!(!m.objects.is_empty(), "backup {key} lists no object");
    for o in &m.objects {
        let Some(e) = blobs.get(&blob_key(&o.sha256)) else { bail!("backup {key} is incomplete: object {} is missing", o.name) };
        ensure!(o.size == 0 || e.size == Some(o.size), "backup {key}: object {} has size {:?}, expected {}", o.name, e.size, o.size);
    }
    Ok(())
}

/// Retention, after the backup `latest` completed: deletes the manifests `retained` does not keep, then the objects no
/// remaining manifest lists. Skipped (an error, nothing deleted) unless the newest backups are complete. Never touches
/// anything outside `home/backup/` and `home/blobs/`. Callers hold `Backups.running`, so no backup runs meanwhile.
async fn prune(store: &Store, env: &UserEnv, latest: &str, now: i64, cache: &mut std::collections::HashMap<String, Vec<u8>>) -> Result<Pruned> {
    let backup_dir = format!("{ROOT}/backup");
    let blob_dir = format!("{ROOT}/blobs/sha256");
    let listed = manifest_keys(store, env, &backup_dir).await?;
    ensure!(listed.iter().any(|k| k == latest), "the new backup {latest} is not listed yet");
    cache.retain(|k, _| listed.contains(k));
    let blobs = store.entries(env, &blob_dir).await?.into_iter().map(|e| (e.key.clone(), e)).collect::<std::collections::HashMap<_, _>>();
    let mut times = vec![];
    for key in &listed {
        let t = match key_time(key) {
            Some(t) => Some(t),
            None => read_manifest(store, env, key, cache).await.ok().and_then(|d| serde_json::from_slice::<Manifest>(&d).ok()).map(|m| m.created_at),
        };
        times.push(t);
    }
    // The two newest backups (by time, whatever the clock says) are never deleted; the backup just written and the
    // one a restore would pick must be complete.
    let mut by_time: Vec<(i64, &String)> = listed.iter().zip(&times).filter_map(|(k, t)| Some(((*t)?, k))).collect();
    by_time.sort_by(|a, b| b.cmp(a));
    let newest: Vec<&String> = by_time.iter().take(2).map(|(_, k)| *k).collect();
    check_complete(store, env, latest, &blobs, cache).await?;
    if let Some(n) = newest.first().filter(|n| n.as_str() != latest) {
        check_complete(store, env, n, &blobs, cache).await?;
    }
    let keep = retained(&times, now);
    let kept: Vec<String> = keep.iter().map(|i| listed[*i].clone()).collect();
    if keep.is_empty() || (listed.len() > 1 && keep.len() < 2) {
        tracing::error!("home backup: refusing to prune, retention keeps {} of {} backups", keep.len(), listed.len());
        bail!("refusing to prune: retention keeps {} of {} backups", keep.len(), listed.len());
    }
    ensure!(kept.iter().any(|k| k == latest) && newest.iter().all(|n| kept.contains(n)), "refusing to prune: the newest backups are not all kept");
    let gone: Vec<String> = listed.iter().enumerate().filter(|(i, _)| !keep.contains(i)).map(|(_, k)| k.clone()).collect();
    // Manifests first: a backup without its manifest is never restored, so a pass stopped halfway leaves only
    // unused objects, which the next pass deletes.
    store.remove_many(env, &backup_dir, &gone).await?;
    for k in &gone {
        cache.remove(k);
    }

    // Objects: from fresh listings, so a manifest that appeared meanwhile still protects its objects.
    let remaining = manifest_keys(store, env, &backup_dir).await?;
    ensure!(remaining.iter().any(|k| k == latest), "the new backup {latest} is no longer listed");
    let mut needed = std::collections::HashSet::new();
    for key in &remaining {
        needed.extend(references(&read_manifest(store, env, key, cache).await?));
    }
    let blobs = store.entries(env, &blob_dir).await?.into_iter().map(|e| (e.key.clone(), e)).collect::<std::collections::HashMap<_, _>>();
    // An object uploaded in the last hour may belong to a backup whose manifest is not written yet.
    let unused: Vec<&Entry> = blobs.values().filter(|e| !needed.contains(&e.key) && e.uploaded_at.is_some_and(|t| now - t >= HOUR_MS)).collect();
    let keys: Vec<String> = unused.iter().map(|e| e.key.clone()).collect();
    store.remove_many(env, &blob_dir, &keys).await?;
    Ok(Pruned { kept, manifests: gone.len(), blobs: keys.len(), bytes: unused.iter().filter_map(|e| e.size).sum() })
}

/// After a complete backup: prunes once the backup released `Backups.running` (never during a backup or another
/// pass). A failure is logged and shown in the backup status; it never fails the backup.
fn spawn_prune(app: &App, store: Store, latest: String) {
    let backups = app.home_backups.clone();
    if backups.prune_queued.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let env = app.env.clone();
    tokio::spawn(async move {
        let _one = backups.running.lock().await;
        backups.prune_queued.store(false, std::sync::atomic::Ordering::SeqCst);
        let mut cache = std::mem::take(&mut *backups.manifests.lock().unwrap());
        let result = prune(&store, &env, &latest, now_ms(), &mut cache).await;
        *backups.manifests.lock().unwrap() = cache;
        let mut info = backups.info.lock().unwrap();
        match result {
            Ok(p) => {
                let names: Vec<&str> = p.kept.iter().map(|k| k.trim_start_matches(&format!("{ROOT}/backup/")).trim_end_matches("/manifest.json")).collect();
                let summary = format!("kept {} backups, deleted {} backups and {} objects ({} MB)", p.kept.len(), p.manifests, p.blobs, p.bytes / 1_000_000);
                if p.manifests > 0 || p.blobs > 0 {
                    tracing::info!("home backup: pruning {summary}; kept {}", names.join(", "));
                }
                info.pruned_at = Some(now_ms());
                info.prune = Some(summary);
                info.prune_error = None;
            }
            Err(e) => {
                tracing::warn!("home backup: pruning skipped: {e:#}");
                info.prune_error = Some(format!("{e:#}"));
            }
        }
    });
}

/// On boot, before the database opens: an empty data folder gets the newest complete backup.
pub async fn restore_if_empty(paths: &crate::paths::Paths, env: &UserEnv) -> Result<Option<i64>> {
    if paths.state_db().exists() {
        return Ok(None);
    }
    // The destination is in config.toml, which may not exist yet either: `LAUNCHPAD_HOME_BACKUP` names it then.
    let configured = std::fs::read_to_string(paths.config_file()).ok().and_then(|t| toml::from_str::<crate::config::Config>(&t).ok()).map(|c| c.home.backup);
    let destination = configured.filter(|d| !d.trim().is_empty()).or_else(|| std::env::var("LAUNCHPAD_HOME_BACKUP").ok());
    let Some(store) = destination.as_deref().and_then(Store::parse) else { return Ok(None) };
    let all = manifests(&store, env, ROOT).await?;
    let Some((key, m)) = all.first() else { return Ok(None) };
    // Complete: every object is there. Only the files restored now are downloaded (and their hashes checked);
    // Home Git bundles and plain blobs are fetched, and checked, when first needed (`restore_bundle`, `fetch_blob`).
    let existing: std::collections::HashSet<String> = store.list(env, &format!("{ROOT}/blobs/sha256")).await?.into_iter().collect();
    if let Some(o) = m.objects.iter().find(|o| !existing.contains(&blob_key(&o.sha256))) {
        bail!("backup {key} is incomplete: object {} is missing", o.name);
    }
    let mut files: Vec<(PathBuf, Vec<u8>)> = vec![];
    for o in &m.objects {
        let Some(path) = restore_path(paths, &o.name) else { continue };
        let data = get_blob(&store, env, &o.sha256).await.map_err(|e| e.context(format!("backup {key}: object {}", o.name)))?;
        if o.name == "config.toml" && data.is_empty() {
            continue;
        }
        files.push((path, data));
    }
    let git: std::collections::BTreeMap<String, String> = m.objects.iter().filter_map(|o| Some((o.name.strip_prefix("git/")?.strip_suffix(".bundle")?.to_string(), o.sha256.clone()))).collect();
    files.push((git_restore_file(paths), serde_json::to_vec(&git)?));
    paths.ensure_dirs()?;
    for (path, data) in files {
        write_atomic(&path, &data)?;
        #[cfg(unix)]
        if path.starts_with(paths.data_dir.join("uploads")) {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        if path == paths.state_db() {
            let key = sealing_key().copied();
            map_secrets(&path, |s| open_secret(key.as_ref(), s)).inspect_err(|_| { let _ = std::fs::remove_file(&path); })?;
        }
    }
    tracing::info!("restored the Home backup {key}");
    Ok(Some(m.created_at))
}

fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("restoring");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Home: a backup every minute when something changed.
pub fn spawn(app: &App) {
    if app.paths.role != crate::role::Role::Home {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let destination = app.config.get().home.backup;
            let result = run(&app, false).await;
            let changed = {
                let mut info = app.home_backups.info.lock().unwrap();
                let before = (info.destination.clone(), info.error.clone(), info.last_complete_at);
                info.destination = Some(destination).filter(|d| !d.trim().is_empty());
                info.error = result.as_ref().err().map(|e| format!("{e:#}"));
                before != (info.destination.clone(), info.error.clone(), info.last_complete_at)
            };
            if let Err(e) = &result {
                tracing::warn!("home backup: {e:#}");
            }
            if changed {
                app.bus.publish("runners", "changed", serde_json::Value::Null);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;
    const MIN: i64 = 60_000;

    /// `retained` as sorted times, for backups given by time.
    fn kept(times: &[Option<i64>], now: i64) -> Vec<Option<i64>> {
        retained(times, now).into_iter().map(|i| times[i]).collect()
    }

    #[test]
    fn retention_keeps_one_backup_per_rule() {
        // A backup every minute for 10 days, newest first.
        let times: Vec<Option<i64>> = (0..10 * 24 * 60).map(|m| Some(NOW - m * MIN)).collect();
        let mut want: Vec<Option<i64>> = [0, MIN, HOUR_MS].into_iter().chain((1..=7).map(|d| d * DAY_MS)).map(|age| Some(NOW - age)).collect();
        want.sort_by_key(|t| std::cmp::Reverse(*t));
        let mut got = kept(&times, NOW);
        got.sort_by_key(|t| std::cmp::Reverse(*t));
        assert_eq!(got, want);
        // Order of the input does not matter.
        let mut shuffled = times.clone();
        shuffled.reverse();
        assert_eq!(retained(&shuffled, NOW).len(), 10);
    }

    #[test]
    fn retention_keeps_the_newest_older_backup_across_gaps() {
        // Nothing between 30 min and 3 days old: the 1 h, 1 d and 2 d rules all pick the 3-day-old backup.
        let times = vec![Some(NOW - MIN), Some(NOW - 2 * MIN), Some(NOW - 30 * MIN), Some(NOW - 3 * DAY_MS), Some(NOW - 3 * DAY_MS - MIN), Some(NOW - 9 * DAY_MS)];
        assert_eq!(kept(&times, NOW), vec![Some(NOW - MIN), Some(NOW - 2 * MIN), Some(NOW - 3 * DAY_MS), Some(NOW - 9 * DAY_MS)]);
        // Exactly one hour old counts as one hour old.
        let times = vec![Some(NOW), Some(NOW - MIN), Some(NOW - 2 * MIN), Some(NOW - HOUR_MS), Some(NOW - HOUR_MS - MIN)];
        assert_eq!(kept(&times, NOW), vec![Some(NOW), Some(NOW - MIN), Some(NOW - HOUR_MS)]);
    }

    #[test]
    fn retention_on_a_fresh_install() {
        assert!(retained(&[], NOW).is_empty());
        assert_eq!(kept(&[Some(NOW)], NOW), vec![Some(NOW)]);
        assert_eq!(kept(&[Some(NOW), Some(NOW - MIN)], NOW), vec![Some(NOW), Some(NOW - MIN)]);
        // A third recent backup is the only one deleted.
        assert_eq!(kept(&[Some(NOW), Some(NOW - MIN), Some(NOW - 2 * MIN)], NOW), vec![Some(NOW), Some(NOW - MIN)]);
    }

    #[test]
    fn retention_with_clock_skew_and_unknown_times() {
        // The clock went back an hour: backups "in the future" are all kept, and so are the two newest.
        let times = vec![Some(NOW + HOUR_MS), Some(NOW + 30 * MIN), Some(NOW + MIN), Some(NOW - MIN), Some(NOW - 2 * MIN), Some(NOW - 2 * HOUR_MS)];
        assert_eq!(kept(&times, NOW), vec![Some(NOW + HOUR_MS), Some(NOW + 30 * MIN), Some(NOW + MIN), Some(NOW - 2 * HOUR_MS)]);
        // Every backup in the future, even far: none deleted.
        let times = vec![Some(NOW + 2 * DAY_MS), Some(NOW + DAY_MS), Some(NOW + HOUR_MS)];
        assert_eq!(retained(&times, NOW).len(), 3);
        // A backup whose time is unknown is kept, and never counts as one of the newest.
        let times = vec![None, Some(NOW), Some(NOW - MIN), Some(NOW - 2 * MIN)];
        assert_eq!(kept(&times, NOW), vec![None, Some(NOW), Some(NOW - MIN)]);
    }

    #[test]
    fn manifest_keys_carry_their_time() {
        let at = 1_762_800_000_123;
        assert_eq!(key_time(&manifest_key(at)), Some(at));
        assert_eq!(key_time("home/backup/latest/manifest.json"), None);
        assert_eq!(key_time("home/backup/20261110T003709.123Z/other.json"), None);
    }

    #[test]
    fn unparsable_manifests_keep_every_hash_they_mention() {
        let h = "ab".repeat(32);
        assert_eq!(references(format!("{{\"objects\": [{{\"sha256\": \"{h}\"").as_bytes()), vec![blob_key(&h)]);
        let m = Manifest { format: 1, created_at: 1, commit: String::new(), objects: vec![Object { name: "state.db".into(), sha256: h.clone(), size: 3 }] };
        assert_eq!(references(&serde_json::to_vec(&m).unwrap()), vec![blob_key(&h)]);
    }

    /// A folder destination with backups at these times (each with its own object, plus one shared object), objects
    /// uploaded `uploaded` ago.
    async fn destination(times: &[i64], uploaded: i64) -> (Store, PathBuf, UserEnv) {
        let dir = std::env::temp_dir().join(format!("launchpad-prune-{}", uuid::Uuid::new_v4()));
        let store = Store::Dir(dir.clone());
        let env = UserEnv::from_vars(Default::default());
        let shared = b"config".to_vec();
        for t in times {
            let data = format!("db {t}").into_bytes();
            let objects = [("state.db", &data), ("config.toml", &shared)].map(|(n, d)| Object { name: n.into(), sha256: hex::encode(Sha256::digest(d)), size: d.len() as u64 });
            for (o, d) in objects.iter().zip([&data, &shared]) {
                put_blob(&store, &env, &dir, &o.sha256, d.clone(), uploaded).await;
            }
            let m = Manifest { format: 1, created_at: *t, commit: String::new(), objects: objects.to_vec() };
            store.put(&env, &manifest_key(*t), serde_json::to_vec(&m).unwrap()).await.unwrap();
        }
        (store, dir, env)
    }

    async fn put_blob(store: &Store, env: &UserEnv, dir: &Path, sha: &str, data: Vec<u8>, uploaded: i64) {
        store.put(env, &blob_key(sha), data).await.unwrap();
        let at = std::time::SystemTime::now() - Duration::from_millis(uploaded as u64);
        std::fs::File::options().write(true).open(dir.join(blob_key(sha))).unwrap().set_modified(at).unwrap();
    }

    fn files(dir: &Path, sub: &str) -> usize {
        walk(&dir.join(sub))
    }

    fn walk(p: &Path) -> usize {
        std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| if e.path().is_dir() { walk(&e.path()) } else { 1 }).sum()).unwrap_or(0)
    }

    #[tokio::test]
    async fn pruning_deletes_old_backups_and_unused_objects() {
        let now = now_ms();
        let times = [now - 2 * MIN, now - MIN, now - 3 * MIN, now - 4 * MIN, now - 2 * HOUR_MS, now - 3 * HOUR_MS];
        let (store, dir, env) = destination(&times, 2 * HOUR_MS).await;
        // Not listed by any manifest: one old, one uploaded a minute ago (a backup in flight).
        put_blob(&store, &env, &dir, &"0".repeat(64), b"old".to_vec(), 2 * HOUR_MS).await;
        put_blob(&store, &env, &dir, &"1".repeat(64), b"new".to_vec(), MIN).await;
        // An unreadable manifest is kept, with every object it mentions.
        put_blob(&store, &env, &dir, &"2".repeat(64), b"odd".to_vec(), 2 * HOUR_MS).await;
        store.put(&env, "home/backup/odd/manifest.json", format!("not json {}", "2".repeat(64)).into_bytes()).await.unwrap();
        // Outside the backups: never touched.
        store.put(&env, "home/credentials/sealed/hf", b"x".to_vec()).await.unwrap();
        store.put(&env, "Work/traces/a.jsonl", b"x".to_vec()).await.unwrap();
        let mut cache = Default::default();
        let p = prune(&store, &env, &manifest_key(now - MIN), now, &mut cache).await.unwrap();
        // Newest, the one before it, the newest at least an hour old, the unreadable one.
        assert_eq!(p.kept, vec![manifest_key(now - 2 * HOUR_MS), manifest_key(now - 2 * MIN), manifest_key(now - MIN), "home/backup/odd/manifest.json".to_string()].into_iter().collect::<std::collections::BTreeSet<_>>().into_iter().collect::<Vec<_>>());
        assert_eq!((p.manifests, p.blobs), (3, 4));
        assert_eq!(files(&dir, "home/backup"), 4);
        // 3 state.db copies, the shared config, the recent unused object and the one the odd manifest names.
        assert_eq!(files(&dir, "home/blobs"), 6);
        assert!(dir.join(blob_key(&"1".repeat(64))).exists() && dir.join(blob_key(&"2".repeat(64))).exists() && !dir.join(blob_key(&"0".repeat(64))).exists());
        assert_eq!(files(&dir, "home/credentials") + files(&dir, "Work"), 2);
        // The backups' own folders went with their manifests.
        assert_eq!(std::fs::read_dir(dir.join("home/backup")).unwrap().count(), 4);
        // A second pass deletes nothing.
        let p = prune(&store, &env, &manifest_key(now - MIN), now, &mut cache).await.unwrap();
        assert_eq!((p.manifests, p.blobs), (0, 0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn pruning_skips_when_the_latest_backup_is_incomplete() {
        let now = now_ms();
        let times = [now - MIN, now - 2 * MIN, now - 3 * MIN, now - 2 * DAY_MS];
        let (store, dir, env) = destination(&times, 2 * HOUR_MS).await;
        put_blob(&store, &env, &dir, &"0".repeat(64), b"old".to_vec(), 2 * HOUR_MS).await;
        let latest = manifest_key(now - MIN);
        let m: Manifest = serde_json::from_slice(&std::fs::read(dir.join(&latest)).unwrap()).unwrap();
        // A missing object, then one with the wrong size.
        let blob = dir.join(blob_key(&m.objects[0].sha256));
        let data = std::fs::read(&blob).unwrap();
        std::fs::remove_file(&blob).unwrap();
        let e = prune(&store, &env, &latest, now, &mut Default::default()).await.unwrap_err();
        assert!(format!("{e:#}").contains("missing"), "{e:#}");
        std::fs::write(&blob, &data[1..]).unwrap();
        let e = prune(&store, &env, &latest, now, &mut Default::default()).await.unwrap_err();
        assert!(format!("{e:#}").contains("size"), "{e:#}");
        // Nothing was deleted.
        assert_eq!(files(&dir, "home/backup"), 4);
        assert!(dir.join(blob_key(&"0".repeat(64))).exists());
        // A backup that is not listed (not written yet) is not pruned after either.
        std::fs::write(&blob, &data).unwrap();
        assert!(prune(&store, &env, &manifest_key(now), now, &mut Default::default()).await.is_err());
        assert_eq!(files(&dir, "home/backup"), 4);
        // The newest backup by time must be complete too, even when it is not the one just written.
        let future = manifest_key(now + HOUR_MS);
        store.put(&env, &future, serde_json::to_vec(&Manifest { objects: vec![Object { name: "state.db".into(), sha256: "3".repeat(64), size: 1 }], ..m.clone() }).unwrap()).await.unwrap();
        assert!(prune(&store, &env, &latest, now, &mut Default::default()).await.is_err());
        assert_eq!(files(&dir, "home/backup"), 5);
        std::fs::remove_file(dir.join(&future)).unwrap();
        // Complete again: it prunes.
        let p = prune(&store, &env, &latest, now, &mut Default::default()).await.unwrap();
        assert_eq!(p.manifests, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sealing() {
        let key = parse_key("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=").unwrap();
        let sealed = seal(&key, b"state").unwrap();
        assert!(sealed.starts_with(MAGIC) && !sealed.windows(5).any(|w| w == b"state"));
        assert_eq!(open(&key, sealed.clone()).unwrap(), b"state");
        // A wrong key never opens a sealed value.
        assert!(open(&[7; 32], sealed).is_err());
        assert!(parse_key("c2hvcnQ=").is_none());
        // Vault secrets inside a backup copy of state.db.
        let sealed = seal_secret(&key, "sk-ant-oat01-abc").unwrap();
        assert!(sealed.starts_with(SEALED_SECRET) && !sealed.contains("sk-ant"));
        assert_eq!(open_secret(Some(&key), &sealed).unwrap(), "sk-ant-oat01-abc");
        assert_eq!(open_secret(None, "plain").unwrap(), "plain");
        assert!(open_secret(None, &sealed).is_err());
    }

    #[test]
    fn destinations() {
        assert_eq!(Store::parse("hf://buckets/me/launchpad/"), Some(Store::Bucket("me/launchpad".into())));
        assert_eq!(Store::parse(" "), None);
        assert!(matches!(Store::parse("/backups/home"), Some(Store::Dir(_))));
    }
}

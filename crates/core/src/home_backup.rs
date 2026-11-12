//! Home's backups: a consistent copy of `state.db` (and `config.toml`) when something changed, and before a
//! planned restart. The minute timer copies `state.db` at most every 10 minutes (`DB_EVERY_MS`: each copy is a full
//! database); new Home Git refs and uploads are backed up every minute, listing the last copy of `state.db` again
//! with the plain blobs and uploads it needs. An empty data folder restores the newest complete backup on boot.
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
//! Other files a restore needs go in as objects too: uploads (`uploads/<sha256>`, `data_files`). Plain folder blobs and handoff files are listed as `plain/<sha256>` and fetched on first use.
//!
//! Retention (`prune`, after each complete backup, one at a time with backups): keep the newest backup, the one
//! before it, the newest one at least an hour old, and the newest one at least 1, 2, …, 7 days old (`retained`);
//! delete the other manifests, then the objects no remaining manifest lists (never one uploaded in the last hour).
//! So the destination holds at most about 10 kept copies of `state.db` plus the copies of the last hour.
//! A failed listing or download is an error, never an empty answer: it stops pruning before it deletes anything,
//! and a boot restore retries, then fails. Pruning also deletes nothing while the listing misses the backup just
//! written or a manifest of the last day does not parse (`check_readable`).
//! `cache.db` is not backed up: it is rebuilt from GitHub.
//!
//! Home Git: each mirror adds a bundle (`git/<folder-id>.bundle`) of its Launchpad branches and kept tips, minus
//! what the folder's remote has (a full bundle without a remote). After a restore, a mirror is cloned from the
//! remote again on first use, then gets its bundles (`git_restore.json` lists them); until then they stay in every
//! backup (`git/<folder-id>.pending.bundle`).

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
        Ok(self.entries(env, prefix).await?.into_iter().map(|e| e.key).collect())
    }

    /// The files under `prefix` (recursive) with their size and upload time. Only a prefix that clearly does not
    /// exist lists as empty: every other failure is an error, never an empty list, since pruning and restore act on
    /// what is missing.
    async fn entries(&self, env: &UserEnv, prefix: &str) -> Result<Vec<Entry>> {
        match self {
            Store::Bucket(b) => {
                let (ok, out, err) = crate::hf::run(env, &["buckets", "ls", &format!("{b}/{prefix}"), "-R", "--format", "json"], None).await?;
                if !ok {
                    // A prefix that does not exist yet, in a bucket that answers.
                    if err.contains("404") && crate::hf::run(env, &["buckets", "info", b], None).await.is_ok_and(|(ok, _, _)| ok) {
                        return Ok(vec![]);
                    }
                    bail!("could not list {prefix} in the backup bucket: {}", crate::hf::last_line(&err));
                }
                let out = out.trim();
                // hf prints nothing, or this, instead of JSON for an empty listing.
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
                        // A prefix that does not exist yet.
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

    pub(crate) async fn remove(&self, env: &UserEnv, key: &str) -> Result<()> {
        match self {
            Store::Bucket(b) => crate::hf::hf(env, &["buckets", "rm", &format!("{b}/{key}"), "--quiet"], None).await.map(|_| ()),
            Store::Dir(d) => Ok(tokio::fs::remove_file(d.join(key)).await?),
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
    /// Plain blobs the last backup could not list: neither on Home's disk nor in the destination.
    pub missing_blobs: Option<String>,
}

#[derive(Clone, Default)]
pub struct Backups {
    info: Arc<Mutex<BackupInfo>>,
    /// `total_changes()` of the state database at the last backup.
    changes: Arc<Mutex<Option<i64>>>,
    /// The last copy of `state.db` and the objects it needs (`run`).
    base: Arc<Mutex<Option<Base>>>,
    running: Arc<tokio::sync::Mutex<()>>,
    /// Per mirror: the hash of its refs and the bundle object saved for them.
    bundles: Arc<Mutex<std::collections::HashMap<String, (String, Object)>>>,
    /// A pruning pass waits for `running`.
    prune_queued: Arc<std::sync::atomic::AtomicBool>,
    /// Manifests read by pruning, by key (they never change).
    manifests: Arc<Mutex<std::collections::HashMap<String, Vec<u8>>>>,
}

/// A copy of `state.db` in the destination: when it was taken, and its object with the plain blobs and uploads
/// listed next to it.
#[derive(Clone)]
struct Base {
    at: i64,
    objects: Vec<Object>,
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

/// The backups a restore can pick under `root`, newest first. A failed listing or download is an error; a manifest
/// that does not parse is left out (pruning reads manifests itself, `check_readable`).
async fn manifests(store: &Store, env: &UserEnv, root: &str) -> Result<Vec<(String, Manifest)>> {
    let mut out = vec![];
    for key in store.list(env, &format!("{root}/backup")).await?.into_iter().filter(|k| k.ends_with("/manifest.json")) {
        let bytes = store.get(env, &key).await?;
        match serde_json::from_slice::<Manifest>(&bytes) {
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

/// What starts a backup (`run`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    /// The minute timer: a new copy of `state.db` at most every `DB_EVERY_MS`; Home Git and uploads every minute.
    Due,
    /// A planned stop: a backup with a new copy of `state.db` when anything changed.
    Changed,
    /// Back up now, Update Home: always, with a new copy of `state.db`.
    Always,
}

/// The shortest time between two copies of `state.db` on the minute timer: each is a full copy (~100 MB), and a
/// busy Home changes its database almost every minute.
const DB_EVERY_MS: i64 = 10 * 60 * 1000;

/// Whether a backup takes a new copy of `state.db`, given whether the database changed since the last copy and
/// when that copy was taken (None: no copy since Home started).
fn new_db_copy(when: When, db_changed: bool, last: Option<i64>, now: i64) -> bool {
    match (when, last) {
        (When::Always, _) | (_, None) => true,
        (When::Changed, Some(_)) => db_changed,
        (When::Due, Some(t)) => db_changed && now - t >= DB_EVERY_MS,
    }
}

/// Takes one backup now if something changed (see `When`). Between two copies of `state.db`, a backup of new Home Git
/// refs or uploads lists the last copy again, with the plain blobs and uploads that copy needs.
pub async fn run(app: &App, when: When) -> Result<bool> {
    let Some(store) = Store::parse(&app.config.get().home.backup) else { return Ok(false) };
    // Before the lock: applying a bundle may first fetch the folder's remote.
    retry_restores(app).await;
    let backups = app.home_backups.clone();
    let _one = backups.running.lock().await;
    let current = app.state_db.call(|c| Ok(c.query_row("SELECT total_changes()", [], |r| r.get::<_, i64>(0))?)).await?;
    let db_changed = *backups.changes.lock().unwrap() != Some(current);
    let base = backups.base.lock().unwrap().clone();
    let fresh = new_db_copy(when, db_changed, base.as_ref().map(|b| b.at), now_ms());
    let git_changed = GIT_CHANGED.swap(false, std::sync::atomic::Ordering::Relaxed);
    if when != When::Always && !fresh && !git_changed {
        return Ok(false);
    }
    let written = backup(app, &store, fresh, base).await;
    let written = match written {
        Ok(w) => w,
        Err(e) => {
            // Not backed up yet: the next minute tries again.
            if git_changed {
                changed();
            }
            return Err(e);
        }
    };
    spawn_prune(app, store.clone(), written);
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

/// Writes one backup and returns its manifest key: a new copy of `state.db` when `fresh`, else `base` again.
async fn backup(app: &App, store: &Store, mut fresh: bool, base: Option<Base>) -> Result<String> {
    let backups = &app.home_backups;
    let existing: std::collections::HashSet<String> = store.list(&app.env, &format!("{ROOT}/blobs/sha256")).await?.into_iter().collect();
    // The last copy must still be in the destination to be listed again.
    let base = base.filter(|b| b.objects.iter().all(|o| existing.contains(&blob_key(&o.sha256))));
    fresh |= base.is_none();
    let mut objects = vec![];
    let mut copied = None;
    if fresh {
        let at = now_ms();
        let (db, changes) = snapshot(app).await?;
        let hash = hex::encode(Sha256::digest(&db));
        let size = db.len() as u64;
        if !existing.contains(&blob_key(&hash)) {
            store.put(&app.env, &blob_key(&hash), db).await?;
        }
        objects.push(Object { name: "state.db".into(), sha256: hash, size });
        copied = Some((at, changes));
    } else if let Some(b) = &base {
        objects.extend(b.objects.iter().filter(|o| o.name == "state.db").cloned());
    }
    let config = tokio::fs::read(app.paths.config_file()).await.unwrap_or_default();
    let hash = hex::encode(Sha256::digest(&config));
    let size = config.len() as u64;
    if !existing.contains(&blob_key(&hash)) {
        store.put(&app.env, &blob_key(&hash), config).await?;
    }
    objects.push(Object { name: "config.toml".into(), sha256: hash, size });
    match bundles(app, store, &existing).await {
        Ok(mut b) => objects.append(&mut b),
        Err(e) => {
            changed();
            return Err(e.context("bundle Home Git"));
        }
    }
    // Plain and asset folders: their contents are already content-addressed. A blob the destination already holds
    // stays listed by hash, whether or not Home's disk has it (not fetched since a restore), so pruning keeps it.
    let mut missing = vec![];
    for sha in crate::plain::live_blobs(app).await? {
        if !existing.contains(&blob_key(&sha)) {
            let data = match tokio::fs::read(crate::plain::blob_path(app, &sha)?).await {
                Ok(d) => d,
                // A listing that lags behind an upload: carry it over from the destination.
                Err(_) => match get_blob(store, &app.env, &sha).await {
                    Ok(d) => d,
                    // Neither on Home's disk nor in the destination: it is gone. Listing it would make every
                    // backup incomplete (no restore), so it is left out and reported.
                    Err(e) => {
                        tracing::error!("home backup: plain blob {sha} is neither on Home nor in the backup destination: {e:#}");
                        missing.push(sha);
                        continue;
                    }
                },
            };
            store.put(&app.env, &blob_key(&sha), data).await?;
        }
        objects.push(Object { name: format!("plain/{sha}"), sha256: sha, size: 0 });
    }
    backups.info.lock().unwrap().missing_blobs = match missing.as_slice() {
        [] => None,
        [one] => Some(format!("the file {one} of a plain folder or handoff is missing on Home and in the backup destination")),
        many => Some(format!("{} files of plain folders or handoffs are missing on Home and in the backup destination (first: {})", many.len(), many[0])),
    };
    for (name, path) in data_files(&app.paths).await {
        let Ok(data) = tokio::fs::read(&path).await else { continue };
        let hash = hex::encode(Sha256::digest(&data));
        let size = data.len() as u64;
        if !existing.contains(&blob_key(&hash)) {
            store.put(&app.env, &blob_key(&hash), data).await?;
        }
        objects.push(Object { name, sha256: hash, size });
    }
    // An older copy of `state.db` may name plain blobs and uploads that are gone from Home since: they stay listed.
    if !fresh && let Some(b) = &base {
        let listed: std::collections::HashSet<String> = objects.iter().map(|o| o.sha256.clone()).collect();
        objects.extend(b.objects.iter().filter(|o| o.name != "state.db" && !listed.contains(&o.sha256)).cloned());
    }
    let now = now_ms();
    let manifest = Manifest { format: 1, created_at: now, commit: crate::commit().into(), objects };
    let written = manifest_key(now);
    store.put(&app.env, &written, serde_json::to_vec_pretty(&manifest)?).await?;
    if let Some((at, changes)) = copied {
        *backups.changes.lock().unwrap() = Some(changes);
        let objects = manifest.objects.iter().filter(|o| o.name == "state.db" || o.name.starts_with("plain/") || o.name.starts_with("uploads/")).cloned().collect();
        *backups.base.lock().unwrap() = Some(Base { at, objects });
    }
    // What a restore brings back: the database as of its copy.
    let at = backups.base.lock().unwrap().as_ref().map(|b| b.at);
    backups.info.lock().unwrap().last_complete_at = at;
    Ok(written)
}

/// Home's files outside `state.db` that a restore needs, by object name: uploads (`uploads/<sha256>`: the
/// attachments of drafts, scheduled launches and routines). What waits for runners that are away is in `state.db`
/// (`runner_delivery`).
async fn data_files(paths: &crate::paths::Paths) -> Vec<(String, PathBuf)> {
    let mut out = vec![];
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

/// `runner-archives.json` of older backups: restored, then imported into `state.db` at start (`runners::spawn`).
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

/// Names of copy states whose last upload to the destination failed (`save_copy_state`).
static COPY_STATE_UNSAVED: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<String>>> = std::sync::LazyLock::new(Default::default);

/// What the readable or trace copies already hold in the destination (`readable-copies.json`,
/// `trace-copies.json`): Home's own file, else the copy kept in the destination (`home/copies/<name>`), so a Home
/// restored on an empty disk does not copy every file again.
pub(crate) async fn load_copy_state(app: &App, store: &Store, name: &str) -> Option<Vec<u8>> {
    if let Ok(data) = tokio::fs::read(app.paths.data_dir.join(name)).await {
        return Some(data);
    }
    store.get(&app.env, &format!("{ROOT}/copies/{name}")).await.ok()
}

/// Saves a copy state on Home's disk and, when it changed (or its last upload failed), in the destination.
pub(crate) async fn save_copy_state(app: &App, store: &Store, name: &str, before: Option<&[u8]>, data: Vec<u8>) -> Result<()> {
    crate::paths::write_private(&app.paths.data_dir.join(name), std::str::from_utf8(&data)?)?;
    let unsaved = COPY_STATE_UNSAVED.lock().unwrap().contains(name);
    if before == Some(&data[..]) && !unsaved {
        return Ok(());
    }
    match store.put(&app.env, &format!("{ROOT}/copies/{name}"), data).await {
        Ok(()) => {
            COPY_STATE_UNSAVED.lock().unwrap().remove(name);
            Ok(())
        }
        Err(e) => {
            COPY_STATE_UNSAVED.lock().unwrap().insert(name.to_string());
            Err(e.context(format!("save {name} in the backup destination")))
        }
    }
}

/// Readable copies next to the backups, for browsing the bucket: `<workspace>/kb/<note>` and
/// `<workspace>/bots/<slug>/<file>`. Only changed files are uploaded and removed files are deleted. Home never reads
/// them back: its own store (in every backup) stays the source.
async fn readable(app: &App, store: &Store) -> Result<()> {
    let before = load_copy_state(app, store, "readable-copies.json").await;
    let mut done: std::collections::BTreeMap<String, String> = before.as_deref().and_then(|d| serde_json::from_slice(d).ok()).unwrap_or_default();
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
    save_copy_state(app, store, "readable-copies.json", before.as_deref(), serde_json::to_vec(&done)?).await?;
    result
}

/// One bundle per mirror: Launchpad branches, kept tips and leftovers the remote does not have.
async fn bundles(app: &App, store: &Store, existing: &std::collections::HashSet<String>) -> Result<Vec<Object>> {
    let mut out = vec![];
    // Read first: a bundle applied while the mirrors are bundled is then in the mirror's own bundle, or still here.
    let pending = read_pending(&app.paths)?;
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
    // A restored mirror may not have its bundle applied yet (not opened yet, or applying failed). Carry the bundle
    // forward, next to the mirror's own, until `restore_bundle` applied it, so pruning never deletes the only copy of
    // its Launchpad branches.
    for (id, hashes) in pending {
        for hash in hashes {
            if out.iter().any(|o| o.sha256 == hash) {
                continue;
            }
            if !existing.contains(&blob_key(&hash)) {
                let data = get_blob(store, &app.env, &hash).await?;
                store.put(&app.env, &blob_key(&hash), data).await?;
            }
            out.push(Object { name: format!("git/{id}.pending.bundle"), sha256: hash, size: 0 });
        }
    }
    Ok(out)
}

/// The Home Git bundles a restore still has to apply, by folder, in the order to apply them (newest first).
type Pending = std::collections::BTreeMap<String, Vec<String>>;

/// The bundles of a backup by folder: the mirror's own (`git/<id>.bundle`), then those it still carried
/// (`git/<id>.pending.bundle`).
fn git_bundles(m: &Manifest) -> Pending {
    let mut out = Pending::new();
    for o in &m.objects {
        let Some(id) = o.name.strip_prefix("git/").and_then(|n| n.strip_suffix(".bundle")) else { continue };
        let hashes = out.entry(id.strip_suffix(".pending").unwrap_or(id).to_string()).or_default();
        if !hashes.contains(&o.sha256) {
            hashes.push(o.sha256.clone());
        }
    }
    out
}

/// `git_restore.json` (no file: nothing pending). An older file names one bundle per folder.
fn read_pending(paths: &crate::paths::Paths) -> Result<Pending> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Hashes {
        One(String),
        Many(Vec<String>),
    }
    let data = match std::fs::read(git_restore_file(paths)) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Pending::new()),
        Err(e) => return Err(e).context("read git_restore.json"),
    };
    let map: std::collections::BTreeMap<String, Hashes> = serde_json::from_slice(&data).context("read git_restore.json")?;
    Ok(map.into_iter().map(|(id, h)| (id, match h { Hashes::One(h) => vec![h], Hashes::Many(v) => v })).filter(|(_, v)| !v.is_empty()).collect())
}

fn write_pending(paths: &crate::paths::Paths, pending: &Pending) -> Result<()> {
    let file = git_restore_file(paths);
    if pending.is_empty() {
        match std::fs::remove_file(&file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    } else {
        write_atomic(&file, &serde_json::to_vec(pending)?)
    }
}

/// Whether a restore still has to apply a bundle to this folder's mirror (true when `git_restore.json` cannot be
/// read, so `restore_bundle` reports why).
pub fn bundle_pending(app: &App, project_id: &str) -> bool {
    git_restore_file(&app.paths).exists() && read_pending(&app.paths).map_or(true, |p| p.contains_key(project_id))
}

/// Home: folders with a bundle still to apply.
fn pending_folders(app: &App) -> Vec<String> {
    read_pending(&app.paths).map(|p| p.into_keys().collect()).unwrap_or_default()
}

/// Every minute: applies the pending bundles of mirrors that exist (an earlier try failed), and forgets those of
/// deleted folders.
async fn retry_restores(app: &App) {
    for id in pending_folders(app) {
        let result = match crate::projects::get(app, &id).await {
            Ok(Some(p)) if crate::home_git::mirror(app, &id).join("HEAD").exists() => crate::home_git::ensure(app, &p).await.map(|_| ()),
            Ok(Some(_)) => Ok(()),
            Ok(None) => forget_bundles(app, &id).await,
            Err(e) => Err(e),
        };
        if let Err(e) = result {
            tracing::warn!("home backup: Home Git bundle of {id}: {e:#}");
        }
    }
}

async fn forget_bundles(app: &App, project_id: &str) -> Result<()> {
    let lock = app.suggestion_locks.get(&format!("home-git-restore:{project_id}"));
    let _lock = lock.lock().await;
    drop_pending(&app.paths, project_id, None)
}

/// One change of `git_restore.json` at a time: folders apply their bundles concurrently.
static PENDING_FILE: Mutex<()> = Mutex::new(());

/// Removes one applied bundle of a folder from `git_restore.json` (all of them with None).
fn drop_pending(paths: &crate::paths::Paths, project_id: &str, hash: Option<&str>) -> Result<()> {
    let _one = PENDING_FILE.lock().unwrap();
    let mut pending = read_pending(paths)?;
    if let Some(left) = pending.get_mut(project_id) {
        left.retain(|h| hash.is_some_and(|hash| h != hash));
        if left.is_empty() {
            pending.remove(project_id);
        }
    }
    write_pending(paths, &pending)
}

/// A restored Home reads plain folder contents it no longer has from the backup destination.
pub async fn fetch_blob(app: &App, sha: &str) -> Result<Vec<u8>> {
    let store = Store::parse(&app.config.get().home.backup).context("no backup destination")?;
    get_blob(&store, &app.env, sha).await
}

fn git_restore_file(paths: &crate::paths::Paths) -> PathBuf {
    paths.data_dir.join("git_restore.json")
}

/// After a restore: applies a mirror's pending bundles once the mirror exists again (cloned from the remote). Each
/// stays pending, and in every backup, until it is applied; callers retry (`home_git::ensure`, `retry_restores`).
pub async fn restore_bundle(app: &App, project_id: &str) -> Result<()> {
    let lock = app.suggestion_locks.get(&format!("home-git-restore:{project_id}"));
    let _lock = lock.lock().await;
    let Some(hashes) = read_pending(&app.paths)?.remove(project_id) else { return Ok(()) };
    let store = Store::parse(&app.config.get().home.backup).context("no backup destination to restore Home Git from")?;
    for hash in hashes {
        let data = get_blob(&store, &app.env, &hash).await.with_context(|| format!("the Home Git bundle of {project_id}"))?;
        let path = app.paths.data_dir.join(format!("restore-{project_id}.bundle"));
        tokio::fs::write(&path, data).await?;
        let result = crate::home_git::apply_bundle(app, project_id, &path).await;
        let _ = tokio::fs::remove_file(&path).await;
        result.context("apply the Home Git bundle (the remote may have lost commits Launchpad branches build on)")?;
        changed();
        drop_pending(&app.paths, project_id, Some(&hash))?;
    }
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

/// A manifest's bytes. Manifests never change once written, so `cache` keeps those that parse between passes (one
/// that does not may be a bad read, read again next time).
async fn read_manifest(store: &Store, env: &UserEnv, key: &str, cache: &mut std::collections::HashMap<String, Vec<u8>>) -> Result<Vec<u8>> {
    if let Some(data) = cache.get(key) {
        return Ok(data.clone());
    }
    let data = store.get(env, key).await.with_context(|| format!("read {key}"))?;
    if serde_json::from_slice::<Manifest>(&data).is_ok() {
        cache.insert(key.to_string(), data.clone());
    }
    Ok(data)
}

/// The youngest an unused object may be to be deleted: one uploaded in the last hour may belong to a backup whose
/// manifest is not written or not listed yet (a backup of this Home cannot be: pruning waits for `Backups.running`;
/// another Home on the same destination, during a Space restart, or a listing that lags behind).
const OBJECT_MIN_AGE_MS: i64 = HOUR_MS;

/// Refuses to prune while a manifest of the last day, or of unknown time, does not parse: it may be a bad read of a
/// backup a restore needs, and the hashes it lists cannot be trusted to protect its objects. Older ones count by
/// the hashes they mention (`references`).
fn check_readable(key: &str, data: &[u8], now: i64) -> Result<()> {
    if serde_json::from_slice::<Manifest>(data).is_err() && key_time(key).is_none_or(|t| now - t <= DAY_MS) {
        bail!("refusing to prune: the recent backup {key} is unreadable");
    }
    Ok(())
}

/// The objects pruning deletes: listed by no manifest in `needed`, and uploaded at least `OBJECT_MIN_AGE_MS` ago (an
/// object of unknown age stays).
fn unused_objects<'a>(blobs: impl IntoIterator<Item = &'a Entry>, needed: &std::collections::HashSet<String>, now: i64) -> Vec<&'a Entry> {
    blobs.into_iter().filter(|e| !needed.contains(&e.key) && e.uploaded_at.is_some_and(|t| now - t >= OBJECT_MIN_AGE_MS)).collect()
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
/// remaining manifest lists. Skipped (an error, nothing deleted) when a listing or download fails, the listing misses
/// `latest`, a recent manifest is unreadable (`check_readable`) or the newest backups are not complete. Never touches
/// anything outside `home/backup/` and `home/blobs/`. Callers hold `Backups.running`, so no backup runs meanwhile.
async fn prune(store: &Store, env: &UserEnv, latest: &str, now: i64, cache: &mut std::collections::HashMap<String, Vec<u8>>) -> Result<Pruned> {
    let backup_dir = format!("{ROOT}/backup");
    let blob_dir = format!("{ROOT}/blobs/sha256");
    let listed = manifest_keys(store, env, &backup_dir).await?;
    ensure!(listed.iter().any(|k| k == latest), "the new backup {latest} is not listed yet");
    cache.retain(|k, _| listed.contains(k));
    for key in listed.iter().filter(|k| key_time(k).is_none_or(|t| now - t <= DAY_MS)) {
        check_readable(key, &read_manifest(store, env, key, cache).await?, now)?;
    }
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
        let data = read_manifest(store, env, key, cache).await?;
        check_readable(key, &data, now)?;
        needed.extend(references(&data));
    }
    let blobs = store.entries(env, &blob_dir).await?;
    let unused = unused_objects(&blobs, &needed, now);
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
    // A destination that cannot be read fails the boot after a few tries: starting empty would hide the backups,
    // and a day later pruning would delete them. Only a destination with no backup starts fresh.
    let mut delay = Duration::from_secs(2);
    let fetched = loop {
        match fetch_newest(paths, &store, env).await {
            Ok(f) => break f,
            Err(e) if delay <= Duration::from_secs(32) => {
                tracing::warn!("home backup: restore failed, trying again in {} s: {e:#}", delay.as_secs());
                tokio::time::sleep(delay).await;
                delay *= 2;
            }
            Err(e) => return Err(e),
        }
    };
    let Some((key, m, mut files)) = fetched else { return Ok(None) };
    // First: once `state.db` exists, the next boot restores nothing, so the pending bundles must be listed by then.
    files.insert(0, (git_restore_file(paths), serde_json::to_vec(&git_bundles(&m))?));
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

/// The newest complete backup and the files a restore writes now, or None when the destination has no backup.
async fn fetch_newest(paths: &crate::paths::Paths, store: &Store, env: &UserEnv) -> Result<Option<(String, Manifest, Vec<(PathBuf, Vec<u8>)>)>> {
    let all = manifests(store, env, ROOT).await?;
    let existing: std::collections::HashSet<String> = store.list(env, &format!("{ROOT}/blobs/sha256")).await?.into_iter().collect();
    let Some((key, m)) = all.into_iter().next() else {
        // Objects without any backup: the listing of backups was wrong, or every manifest is damaged.
        ensure!(existing.is_empty(), "the backup destination holds backup objects but no readable backup under {ROOT}/backup: fix it, or empty {ROOT}/ to start a new Home");
        return Ok(None);
    };
    // Complete: every object is there. Only the files restored now are downloaded (and their hashes checked);
    // Home Git bundles and plain blobs are fetched, and checked, when first needed (`restore_bundle`, `fetch_blob`).
    if let Some(o) = m.objects.iter().find(|o| !existing.contains(&blob_key(&o.sha256))) {
        bail!("backup {key} is incomplete: object {} is missing", o.name);
    }
    let mut files: Vec<(PathBuf, Vec<u8>)> = vec![];
    for o in &m.objects {
        let Some(path) = restore_path(paths, &o.name) else { continue };
        let data = get_blob(store, env, &o.sha256).await.map_err(|e| e.context(format!("backup {key}: object {}", o.name)))?;
        if o.name == "config.toml" && data.is_empty() {
            continue;
        }
        files.push((path, data));
    }
    Ok(Some((key, m, files)))
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

/// Home: a backup every minute when something changed (a new copy of `state.db` at most every `DB_EVERY_MS`).
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
            let result = run(&app, When::Due).await;
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
        let (store, dir, env) = destination(&times, 2 * DAY_MS).await;
        // Not listed by any manifest: one old, one uploaded a minute ago (a backup in flight), one uploaded two hours
        // ago (an older copy of state.db: older than the grace hour).
        put_blob(&store, &env, &dir, &"0".repeat(64), b"old".to_vec(), 2 * DAY_MS).await;
        put_blob(&store, &env, &dir, &"1".repeat(64), b"new".to_vec(), MIN).await;
        put_blob(&store, &env, &dir, &"3".repeat(64), b"hours".to_vec(), 2 * HOUR_MS).await;
        // An unreadable manifest older than a day counts by every object it mentions (here the daily backup it is).
        let odd = manifest_key(now - 2 * DAY_MS);
        put_blob(&store, &env, &dir, &"2".repeat(64), b"odd".to_vec(), 2 * DAY_MS).await;
        store.put(&env, &odd, format!("not json {}", "2".repeat(64)).into_bytes()).await.unwrap();
        // Outside the backups: never touched.
        store.put(&env, "home/credentials/sealed/hf", b"x".to_vec()).await.unwrap();
        store.put(&env, "Work/traces/a.jsonl", b"x".to_vec()).await.unwrap();
        let mut cache = Default::default();
        let p = prune(&store, &env, &manifest_key(now - MIN), now, &mut cache).await.unwrap();
        // Newest, the one before it, the newest at least an hour old, the newest at least a day old.
        assert_eq!(p.kept, vec![manifest_key(now - 2 * HOUR_MS), manifest_key(now - 2 * MIN), manifest_key(now - MIN), odd].into_iter().collect::<std::collections::BTreeSet<_>>().into_iter().collect::<Vec<_>>());
        assert_eq!((p.manifests, p.blobs), (3, 5));
        assert_eq!(files(&dir, "home/backup"), 4);
        // 3 state.db copies, the shared config, the unused object of the last hour and the one the odd manifest names.
        assert_eq!(files(&dir, "home/blobs"), 6);
        assert!(["1", "2"].iter().all(|h| dir.join(blob_key(&h.repeat(64))).exists()) && ["0", "3"].iter().all(|h| !dir.join(blob_key(&h.repeat(64))).exists()));
        assert_eq!(files(&dir, "home/credentials") + files(&dir, "Work"), 2);
        // The backups' own folders went with their manifests.
        assert_eq!(std::fs::read_dir(dir.join("home/backup")).unwrap().count(), 4);
        // An unreadable manifest of the last day (a bad read?) holds pruning back.
        let recent = manifest_key(now - 5 * MIN);
        store.put(&env, &recent, b"{\"objects\": [".to_vec()).await.unwrap();
        let e = prune(&store, &env, &manifest_key(now - MIN), now, &mut cache).await.unwrap_err();
        assert!(format!("{e:#}").contains("unreadable"), "{e:#}");
        assert_eq!(files(&dir, "home/backup"), 5);
        std::fs::remove_file(dir.join(&recent)).unwrap();
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
    fn pruning_deletes_only_old_unneeded_objects() {
        let entry = |hash: &str, at: Option<i64>| Entry { key: blob_key(hash), size: Some(1), uploaded_at: at };
        let blobs = [entry("aa", Some(NOW - 5 * DAY_MS)), entry("bb", Some(NOW - 5 * DAY_MS)), entry("cc", Some(NOW - 30 * MIN)), entry("dd", None), entry("ee", Some(NOW - HOUR_MS))];
        let needed = std::collections::HashSet::from([blob_key("aa")]);
        // Unneeded objects at least an hour old go; a needed one, one younger than an hour and one of unknown age stay.
        assert_eq!(unused_objects(&blobs, &needed, NOW).iter().map(|e| e.key.clone()).collect::<Vec<_>>(), vec![blob_key("bb"), blob_key("ee")]);
        // An unreadable manifest of the last day (or of unknown time) holds pruning back; an older one does not.
        assert!(check_readable(&manifest_key(NOW - 2000), b"{\"objects\": [", NOW).is_err());
        assert!(check_readable("home/backup/x/manifest.json", b"{", NOW).is_err());
        assert!(check_readable(&manifest_key(NOW - 2 * DAY_MS), b"{", NOW).is_ok());
        let m = Manifest { format: 1, created_at: NOW, commit: String::new(), objects: vec![] };
        assert!(check_readable(&manifest_key(NOW - 2000), &serde_json::to_vec(&m).unwrap(), NOW).is_ok());
    }

    #[test]
    fn state_db_is_copied_at_most_every_ten_minutes() {
        // The first backup since Home started always copies it.
        assert!(new_db_copy(When::Due, false, None, NOW));
        // The timer: only a changed database, and only ten minutes after the last copy.
        assert!(!new_db_copy(When::Due, false, Some(NOW - HOUR_MS), NOW));
        assert!(!new_db_copy(When::Due, true, Some(NOW - 9 * MIN), NOW));
        assert!(new_db_copy(When::Due, true, Some(NOW - 10 * MIN), NOW));
        // A planned stop copies any change at once; Back up now and Update Home always copy.
        assert!(new_db_copy(When::Changed, true, Some(NOW - MIN), NOW));
        assert!(!new_db_copy(When::Changed, false, Some(NOW - MIN), NOW));
        assert!(new_db_copy(When::Always, false, Some(NOW - MIN), NOW));
    }

    #[test]
    fn pending_bundles_by_folder() {
        let mut m = Manifest { format: 1, created_at: 0, commit: String::new(), objects: vec![] };
        for (name, sha) in [("state.db", "s"), ("git/f1.bundle", "new"), ("git/f1.pending.bundle", "old"), ("git/f2.pending.bundle", "p"), ("git/f1.pending.bundle", "new")] {
            m.objects.push(Object { name: name.into(), sha256: sha.into(), size: 0 });
        }
        let p = git_bundles(&m);
        assert_eq!(p["f1"], vec!["new".to_string(), "old".to_string()]);
        assert_eq!(p["f2"], vec!["p".to_string()]);
        assert_eq!(p.len(), 2);
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

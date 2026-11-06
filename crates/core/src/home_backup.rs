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
//! Pruning keeps every backup of the last day (and always the newest), then removes objects no kept manifest
//! needs. `cache.db` is not backed up: it is rebuilt from GitHub.
//!
//! Home Git: each mirror adds a bundle (`git/<folder-id>.bundle`) of its Launchpad branches and kept tips, minus
//! what the folder's remote has (a full bundle without a remote). After a restore, a mirror is cloned from the
//! remote again on first use, then gets its bundle (`git_restore.json` lists them).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ts_rs::TS;

use crate::App;
use crate::shellenv::UserEnv;
use crate::util::now_ms;

const KEEP_MS: i64 = 24 * 3600 * 1000;

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
}

#[derive(Clone, Default)]
pub struct Backups {
    info: Arc<Mutex<BackupInfo>>,
    /// `total_changes()` of the state database at the last backup.
    changes: Arc<Mutex<Option<i64>>>,
    running: Arc<tokio::sync::Mutex<()>>,
    /// Per mirror: the hash of its refs and the bundle object saved for them.
    bundles: Arc<Mutex<std::collections::HashMap<String, (String, Object)>>>,
}

static GIT_CHANGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Home Git changed: the next backup runs even if the database did not.
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

/// Which manifests pruning removes: those older than a day, except the newest.
fn prunable(manifests: &[(String, Manifest)], now: i64) -> Vec<String> {
    manifests.iter().skip(1).filter(|(_, m)| now - m.created_at > KEEP_MS).map(|(k, _)| k.clone()).collect()
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
    let now = now_ms();
    let manifest = Manifest { format: 1, created_at: now, commit: crate::commit().into(), objects };
    store.put(&app.env, &manifest_key(now), serde_json::to_vec_pretty(&manifest)?).await?;
    *backups.changes.lock().unwrap() = Some(changes);
    backups.info.lock().unwrap().last_complete_at = Some(now);
    if let Err(e) = prune(&store, &app.env, now).await {
        tracing::warn!("home backup: pruning: {e:#}");
    }
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

/// Top-level names the bucket already uses (Home backups, the Space connector, older trace exports).
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

async fn prune(store: &Store, env: &UserEnv, now: i64) -> Result<()> {
    let all = manifests(store, env, ROOT).await?;
    let gone = prunable(&all, now);
    for key in &gone {
        store.remove(env, key).await?;
    }
    let needed: std::collections::HashSet<String> = all.iter().filter(|(k, _)| !gone.contains(k)).flat_map(|(_, m)| m.objects.iter().map(|o| blob_key(&o.sha256))).collect();
    for key in store.list(env, &format!("{ROOT}/blobs/sha256")).await? {
        if !needed.contains(&key) {
            store.remove(env, &key).await?;
        }
    }
    Ok(())
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
    let mut files: Vec<(PathBuf, Vec<u8>)> = vec![];
    for o in &m.objects {
        let data = get_blob(&store, env, &o.sha256).await.map_err(|e| e.context(format!("backup {key}: object {}", o.name)))?;
        let path = match o.name.as_str() {
            "state.db" => paths.state_db(),
            // Settings edited since (or set by the Space) win over the backup.
            "config.toml" if !paths.config_file().exists() && !data.is_empty() => paths.config_file(),
            _ => continue,
        };
        files.push((path, data));
    }
    let git: std::collections::BTreeMap<String, String> = m.objects.iter().filter_map(|o| Some((o.name.strip_prefix("git/")?.strip_suffix(".bundle")?.to_string(), o.sha256.clone()))).collect();
    files.push((git_restore_file(paths), serde_json::to_vec(&git)?));
    paths.ensure_dirs()?;
    for (path, data) in files {
        write_atomic(&path, &data)?;
        if path == paths.state_db() {
            let key = sealing_key().copied();
            map_secrets(&path, |s| open_secret(key.as_ref(), s)).inspect_err(|_| { let _ = std::fs::remove_file(&path); })?;
        }
    }
    tracing::info!("restored the Home backup {key}");
    Ok(Some(m.created_at))
}

fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
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

    fn manifest(at: i64) -> (String, Manifest) {
        (manifest_key(at), Manifest { format: 1, created_at: at, commit: String::new(), objects: vec![] })
    }

    #[test]
    fn pruning_keeps_the_last_day_and_the_newest() {
        let day = KEEP_MS;
        let now = 10 * day;
        let fresh = vec![manifest(now - 1000), manifest(now - day + 1000), manifest(now - 2 * day)];
        assert_eq!(prunable(&fresh, now), vec![manifest_key(now - 2 * day)]);
        // Only old backups: the newest stays.
        let old = vec![manifest(now - 3 * day), manifest(now - 4 * day)];
        assert_eq!(prunable(&old, now), vec![manifest_key(now - 4 * day)]);
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

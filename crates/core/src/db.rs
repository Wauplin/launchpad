//! SQLite behind a small actor: one thread owns each connection, callers send closures.
//!
//! - `state.db`: your data. Schema changes are SQL statements applied in order with `PRAGMA user_version`.
//! - `cache.db`: GitHub mirror. If the schema hash changes, the file is deleted and rebuilt.

use std::path::Path;
use std::sync::mpsc;

use anyhow::{Context, Result, anyhow};
use rusqlite::Connection;
use sha2::{Digest, Sha256};

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

#[derive(Clone)]
pub struct Db {
    tx: mpsc::Sender<Job>,
}

impl Db {
    fn spawn(mut conn: Connection, name: &str) -> Db {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name(format!("db-{name}"))
            .spawn(move || {
                for job in rx {
                    job(&mut conn);
                }
            })
            .expect("spawn db thread");
        Db { tx }
    }

    /// Runs `f` on the database thread.
    pub async fn call<R, F>(&self, f: F) -> Result<R>
    where
        R: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<R> + Send + 'static,
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(Box::new(move |conn| {
                let _ = tx.send(f(conn));
            }))
            .map_err(|_| anyhow!("database thread stopped"))?;
        rx.await.map_err(|_| anyhow!("database thread stopped"))?
    }
}

fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;",
    )?;
    Ok(conn)
}

/// `state.db` schema, one entry per version. Append only; never edit an entry that already ran.
const STATE_MIGRATIONS: &[&str] = &[
    // 1: projects, sessions, events, launch drafts
    "CREATE TABLE project (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        path TEXT NOT NULL UNIQUE,
        remote_url TEXT,
        gh_repo TEXT,
        description TEXT NOT NULL DEFAULT '',
        is_temp INTEGER NOT NULL DEFAULT 0,
        pinned INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL
    );
    CREATE TABLE session (
        id TEXT PRIMARY KEY,
        harness TEXT NOT NULL,
        harness_session_id TEXT,
        project_id TEXT,
        cwd TEXT NOT NULL,
        title TEXT NOT NULL,
        status TEXT NOT NULL,
        model TEXT,
        effort TEXT,
        launch_spec_json TEXT NOT NULL,
        prompt TEXT NOT NULL,
        predraft_id TEXT,
        cost_usd REAL,
        usage_json TEXT,
        started_at INTEGER NOT NULL,
        ended_at INTEGER
    );
    CREATE TABLE session_event (
        session_id TEXT NOT NULL,
        seq INTEGER NOT NULL,
        ts INTEGER NOT NULL,
        kind TEXT NOT NULL,
        json TEXT NOT NULL,
        PRIMARY KEY (session_id, seq)
    );
    CREATE TABLE session_raw (
        session_id TEXT NOT NULL,
        n INTEGER NOT NULL,
        ts INTEGER NOT NULL,
        dir TEXT NOT NULL,
        line TEXT NOT NULL,
        PRIMARY KEY (session_id, n)
    );
    CREATE TABLE launch_draft (
        id TEXT PRIMARY KEY,
        spec_json TEXT NOT NULL,
        updated_at INTEGER NOT NULL
    );",
    // 2: fork upstreams, session status details
    "ALTER TABLE project ADD COLUMN gh_upstream TEXT;
    ALTER TABLE session ADD COLUMN status_since INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE session ADD COLUMN open_question INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE session ADD COLUMN last_seq INTEGER NOT NULL DEFAULT 0;
    CREATE INDEX session_started ON session (started_at);",
    // 3: durable inbox triage overrides, scoped to a notification update time.
    "CREATE TABLE inbox_override (id TEXT PRIMARY KEY, updated_at TEXT NOT NULL, action TEXT NOT NULL);",
    // 4: inbox membership for threads observed unread or opened locally.
    "CREATE TABLE inbox_active (id TEXT PRIMARY KEY);",
];

pub fn open_state(path: &Path) -> Result<Db> {
    let mut conn = open(path)?;
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for (i, sql) in STATE_MIGRATIONS.iter().enumerate().skip(version as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql).with_context(|| format!("state.db migration {}", i + 1))?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
        tracing::info!("state.db: applied migration {}", i + 1);
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS scheduled_launch (id TEXT PRIMARY KEY, json TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS draft_comment (id TEXT PRIMARY KEY, json TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS predraft (id TEXT PRIMARY KEY, json TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS cron_agent (id TEXT PRIMARY KEY, json TEXT NOT NULL);",
    )?;
    Ok(Db::spawn(conn, "state"))
}

/// `cache.db` schema. Any change to this text rebuilds the cache from scratch.
const CACHE_SCHEMA: &str = "
CREATE TABLE gh_response (url TEXT PRIMARY KEY, etag TEXT, last_modified TEXT, json TEXT NOT NULL);
CREATE TABLE inbox_thread (id TEXT PRIMARY KEY, updated_at TEXT NOT NULL, json TEXT NOT NULL);
CREATE TABLE inbox_subscription (id TEXT PRIMARY KEY, subscribed INTEGER, ignored INTEGER, updated_at TEXT NOT NULL, checked_at INTEGER NOT NULL);
CREATE TABLE inbox_preview (id TEXT PRIMARY KEY, fetched_at INTEGER NOT NULL, json TEXT NOT NULL);
CREATE TABLE gh_notification (id TEXT PRIMARY KEY, repo TEXT, subject_type TEXT, subject_number INTEGER,
    reason TEXT, unread INTEGER, updated_at TEXT, last_read_at TEXT, json TEXT NOT NULL);
CREATE TABLE gh_item (repo TEXT, number INTEGER, kind TEXT, state TEXT, is_draft INTEGER, title TEXT,
    author TEXT, updated_at TEXT, fetched_at INTEGER, json TEXT NOT NULL, PRIMARY KEY (repo, number));
CREATE TABLE gh_timeline (repo TEXT, number INTEGER, fetched_at INTEGER, json TEXT NOT NULL,
    PRIMARY KEY (repo, number));
CREATE TABLE gh_diff (repo TEXT, number INTEGER, base_sha TEXT, head_sha TEXT, json TEXT NOT NULL,
    PRIMARY KEY (repo, number, head_sha));
CREATE TABLE gh_etag (url TEXT PRIMARY KEY, etag TEXT, last_modified TEXT, fetched_at INTEGER);
CREATE TABLE image_cache (url TEXT PRIMARY KEY, path TEXT, content_type TEXT, fetched_at INTEGER);
";

pub fn open_cache(path: &Path) -> Result<Db> {
    let hash = hex::encode(Sha256::digest(CACHE_SCHEMA.as_bytes()));
    let current: Option<String> = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .ok()
        .and_then(|c| c.query_row("SELECT value FROM meta WHERE key = 'schema_hash'", [], |r| r.get(0)).ok());
    if current.as_deref() != Some(hash.as_str()) {
        if path.exists() {
            tracing::info!("cache.db: schema changed, rebuilding");
        }
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
        let conn = open(path)?;
        conn.execute_batch(CACHE_SCHEMA)?;
        conn.execute_batch("CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);")?;
        conn.execute("INSERT INTO meta (key, value) VALUES ('schema_hash', ?1)", [&hash])?;
        return Ok(Db::spawn(conn, "cache"));
    }
    Ok(Db::spawn(open(path)?, "cache"))
}

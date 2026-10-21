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
        self.enqueue(f).await
    }

    /// Queues immediately, before the returned future is polled. Call while holding the lock
    /// protecting a snapshot when database writes must follow the same mutation order.
    pub fn enqueue<R, F>(&self, f: F) -> impl std::future::Future<Output = Result<R>> + use<R, F>
    where
        R: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<R> + Send + 'static,
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let sent = self.tx.send(Box::new(move |conn| {
            let _ = tx.send(f(conn));
        }));
        async move {
            sent.map_err(|_| anyhow!("database thread stopped"))?;
            rx.await.map_err(|_| anyhow!("database thread stopped"))?
        }
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
    // 5: independently revocable browser devices (only credential hashes are persisted).
    "CREATE TABLE browser_device (
        id TEXT PRIMARY KEY, secret_hash TEXT NOT NULL UNIQUE, name TEXT NOT NULL,
        created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL
    );",
    // 6: weekly quota baselines and observed usage drops, independent of reset predictions.
    "CREATE TABLE quota_weekly_reading (
        harness TEXT PRIMARY KEY, observed_at INTEGER NOT NULL,
        used_percent REAL NOT NULL, resets_at INTEGER
    );
    CREATE TABLE quota_weekly_reset (
        harness TEXT NOT NULL, observed_at INTEGER NOT NULL, previous_at INTEGER NOT NULL,
        previous_percent REAL NOT NULL, used_percent REAL NOT NULL,
        PRIMARY KEY (harness, observed_at)
    );
    CREATE INDEX quota_weekly_reset_time ON quota_weekly_reset (observed_at);",
    // 7: a weekly reset needs a large usage drop; remove earlier one-point reading noise.
    "DELETE FROM quota_weekly_reset
        WHERE previous_percent - used_percent < 10.0
          AND NOT (used_percent <= 1.0 AND previous_percent >= 5.0);",

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
CREATE TABLE IF NOT EXISTS cron_agent (id TEXT PRIMARY KEY, json TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS workspace (id TEXT PRIMARY KEY, json TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS task_worktree (id TEXT PRIMARY KEY, json TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS browser_operation (id TEXT PRIMARY KEY, path TEXT NOT NULL, request_hash TEXT NOT NULL, status INTEGER, response TEXT);
CREATE TABLE IF NOT EXISTS review_state (key TEXT PRIMARY KEY, json TEXT NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS browser_device (id TEXT PRIMARY KEY, secret_hash TEXT NOT NULL UNIQUE, name TEXT NOT NULL, created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS host (id TEXT PRIMARY KEY, json TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS push_subscription (endpoint TEXT PRIMARY KEY, device_id TEXT NOT NULL, p256dh TEXT NOT NULL, auth TEXT NOT NULL, titles INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL);",
    )?;
    // Some development databases retain schema numbers from reverted work. Check these
    // additive columns directly, like the unversioned tables above.
    let columns = conn.prepare("PRAGMA table_info(session)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let has = |name: &str| columns.iter().any(|c| c == name);
    let project_columns = conn.prepare("PRAGMA table_info(project)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if !project_columns.iter().any(|c| c == "worktree_setup") { conn.execute_batch("ALTER TABLE project ADD COLUMN worktree_setup TEXT;")?; }
    if !project_columns.iter().any(|c| c == "merge_when_done") { conn.execute_batch("ALTER TABLE project ADD COLUMN merge_when_done INTEGER NOT NULL DEFAULT 0;")?; }
    let device_columns = conn.prepare("PRAGMA table_info(browser_device)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if !device_columns.iter().any(|c| c == "last_seen_at") { conn.execute_batch("ALTER TABLE browser_device ADD COLUMN last_seen_at INTEGER;")?; }
    if !has("quota_pause_json") { conn.execute_batch("ALTER TABLE session ADD COLUMN quota_pause_json TEXT;")?; }
    if !has("title_pending") { conn.execute_batch("ALTER TABLE session ADD COLUMN title_pending INTEGER;")?; }
    // Existing sessions start as seen, so old Done sessions do not all need you at once.
    if !has("seen_at") { conn.execute_batch("ALTER TABLE session ADD COLUMN seen_at INTEGER NOT NULL DEFAULT 0; UPDATE session SET seen_at = status_since;")?; }
    if !has("last_message") {
        conn.execute_batch(
            "ALTER TABLE session ADD COLUMN last_message TEXT;
             UPDATE session SET last_message = (SELECT substr(json_extract(e.json, '$.text'), 1, 400) FROM session_event e
                 WHERE e.session_id = session.id AND e.kind = 'assistant_message' ORDER BY e.seq DESC LIMIT 1);",
        )?;
    }
    if !has("archived_done") {
        let tx = conn.transaction()?;
        tx.execute_batch(
            "ALTER TABLE session ADD COLUMN archived_done INTEGER NOT NULL DEFAULT 0;
             UPDATE session SET archived_done = coalesce((
                 SELECT json_extract(e.json, '$.status') = 'waiting' FROM session_event e
                 WHERE e.session_id = session.id AND e.kind = 'status'
                   AND json_extract(e.json, '$.status') != 'closed'
                 ORDER BY e.seq DESC LIMIT 1), 0)
             WHERE status = 'closed';",
        )?;
        tx.commit()?;
    }
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

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn archived_completion_backfill_uses_the_latest_turn_and_survives_reopen() {
        let path = std::env::temp_dir().join(format!("launchpad-archive-{}.db", uuid::Uuid::new_v4()));
        let db = super::open_state(&path).unwrap();
        db.call(|c| {
            c.execute_batch("ALTER TABLE session DROP COLUMN archived_done;")?;
            for (id, status) in [("done", "closed"), ("stopped", "closed"),
                ("resumed", "closed"), ("unknown", "closed"), ("active", "working")] {
                c.execute("INSERT INTO session (id, harness, cwd, title, status, launch_spec_json, prompt, started_at)
                    VALUES (?1, 'codex', '/tmp', 'task', ?2, '{}', 'task', 0)", [id, status])?;
            }
            for (id, states) in [("done", vec!["waiting", "closed"]),
                ("stopped", vec!["stopped", "closed"]),
                ("resumed", vec!["waiting", "closed", "working", "closed"]),
                ("active", vec!["waiting", "working"])] {
                for (seq, status) in states.iter().enumerate() {
                    c.execute("INSERT INTO session_event VALUES (?1, ?2, 0, 'status', ?3)",
                        rusqlite::params![id, seq as i64, serde_json::json!({"kind": "status", "status": status}).to_string()])?;
                }
            }
            Ok(())
        }).await.unwrap();
        drop(db);
        for _ in 0..2 {
            let db = super::open_state(&path).unwrap();
            db.call(|c| {
                for id in ["done", "stopped", "resumed", "unknown", "active"] {
                    let done: bool = c.query_row("SELECT archived_done FROM session WHERE id = ?1", [id], |r| r.get(0))?;
                    assert_eq!(done, id == "done", "{id}");
                }
                Ok(())
            }).await.unwrap();
        }
        std::fs::remove_file(path).unwrap();
    }
}

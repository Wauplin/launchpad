//! The ledger of a workspace's memory: what was given to Funes, in a SQLite file beside the memory
//! (`<data>/recall/<workspace>/ledger.db`). It is derived like the memory and goes with it on a rebuild.
//!
//! Per session: the Home sequence its events are indexed through (the cursor), the next Funes `seq` (Funes wants a
//! dense counter per session) and why it is not indexed when it is not. Per turn: its Funes `seq` and the Home
//! sequences of its events, which turn a hit (`session`, Funes `seq`) into `lp:session/<id>/<seq>`.
//!
//! A session's row moves only after Funes accepted its turns, so an interrupted pass gives the same turns again
//! (same IDs: Funes drops them) and a finished one gives nothing: indexing is idempotent and resumable.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct Ledger {
    path: PathBuf,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionRow {
    pub cursor: i64,
    pub next_fseq: i64,
    pub turns: i64,
    /// Why the session is left out (`archived`, `hidden`, `empty`, `unconvertible`), if it is.
    pub skip: Option<String>,
    pub error: Option<String>,
    /// Passes that failed in a row.
    pub failures: i64,
}

/// One indexed turn: its Funes `seq`, the Home sequences of its first and last events, its time (Unix ms).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TurnRow {
    pub fseq: i64,
    pub lp_seq: i64,
    pub lp_end: i64,
    pub ts: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Totals {
    /// Sessions with at least one turn in the memory, and their turns and blocks.
    pub sessions: i64,
    pub turns: i64,
    pub blocks: i64,
    /// Blocks cut to the size limit (`too large`): the memory holds their start and end.
    pub truncated_blocks: i64,
    /// Sessions left out, by reason.
    pub skipped: BTreeMap<String, i64>,
    /// Sessions whose last pass failed.
    pub failed: i64,
    /// The time of the newest indexed turn (Unix ms, 0: none).
    pub newest_turn: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Failure {
    pub session_id: String,
    pub error: String,
    pub failures: i64,
}

impl Ledger {
    pub fn open(dir: &Path) -> Result<Ledger> {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let ledger = Ledger { path: dir.join("ledger.db") };
        ledger.conn()?.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS session (
                 session_id TEXT PRIMARY KEY,
                 cursor INTEGER NOT NULL DEFAULT 0,
                 next_fseq INTEGER NOT NULL DEFAULT 0,
                 turns INTEGER NOT NULL DEFAULT 0,
                 blocks INTEGER NOT NULL DEFAULT 0,
                 truncated INTEGER NOT NULL DEFAULT 0,
                 skip TEXT,
                 error TEXT,
                 failures INTEGER NOT NULL DEFAULT 0,
                 updated_at INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS turn (
                 session_id TEXT NOT NULL,
                 fseq INTEGER NOT NULL,
                 lp_seq INTEGER NOT NULL,
                 lp_end INTEGER NOT NULL,
                 ts INTEGER NOT NULL,
                 PRIMARY KEY (session_id, fseq)
             );",
        )?;
        Ok(ledger)
    }

    fn conn(&self) -> Result<Connection> {
        let c = Connection::open(&self.path).with_context(|| format!("opening {}", self.path.display()))?;
        c.busy_timeout(std::time::Duration::from_secs(10))?;
        Ok(c)
    }

    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self.conn()?.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).optional()?)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn()?.execute("INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value", params![key, value])?;
        Ok(())
    }

    pub fn sessions(&self) -> Result<BTreeMap<String, SessionRow>> {
        let c = self.conn()?;
        let mut st = c.prepare("SELECT session_id, cursor, next_fseq, turns, skip, error, failures FROM session")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, SessionRow { cursor: r.get(1)?, next_fseq: r.get(2)?, turns: r.get(3)?, skip: r.get(4)?, error: r.get(5)?, failures: r.get(6)? })))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn session(&self, id: &str) -> Result<SessionRow> {
        let row = self.conn()?.query_row("SELECT cursor, next_fseq, turns, skip, error, failures FROM session WHERE session_id = ?1", [id], |r| {
            Ok(SessionRow { cursor: r.get(0)?, next_fseq: r.get(1)?, turns: r.get(2)?, skip: r.get(3)?, error: r.get(4)?, failures: r.get(5)? })
        });
        Ok(row.optional()?.unwrap_or_default())
    }

    /// Funes accepted these turns of a session: they are recorded and its cursor moves to `cursor`. `turns` start
    /// at the session's `next_fseq`. A session with no turn at all so far is recorded as `empty`.
    pub fn commit(&self, id: &str, cursor: i64, turns: &[TurnRow], blocks: i64, truncated: i64, now: i64) -> Result<()> {
        let mut c = self.conn()?;
        let tx = c.transaction()?;
        tx.execute("INSERT OR IGNORE INTO session (session_id) VALUES (?1)", [id])?;
        for t in turns {
            tx.execute("INSERT OR REPLACE INTO turn (session_id, fseq, lp_seq, lp_end, ts) VALUES (?1, ?2, ?3, ?4, ?5)", params![id, t.fseq, t.lp_seq, t.lp_end, t.ts])?;
        }
        let next = turns.last().map(|t| t.fseq + 1);
        tx.execute(
            "UPDATE session SET cursor = MAX(cursor, ?2), next_fseq = COALESCE(?3, next_fseq), turns = turns + ?4, blocks = blocks + ?5, truncated = truncated + ?6,
                 error = NULL, failures = 0, updated_at = ?7 WHERE session_id = ?1",
            params![id, cursor, next, turns.len() as i64, blocks, truncated, now],
        )?;
        tx.execute("UPDATE session SET skip = CASE WHEN turns = 0 THEN 'empty' ELSE NULL END WHERE session_id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    /// The session is left out for `reason` (None: no longer).
    pub fn set_skip(&self, id: &str, reason: Option<&str>, now: i64) -> Result<()> {
        let c = self.conn()?;
        c.execute("INSERT OR IGNORE INTO session (session_id) VALUES (?1)", [id])?;
        c.execute("UPDATE session SET skip = ?2, updated_at = ?3 WHERE session_id = ?1", params![id, reason, now])?;
        Ok(())
    }

    pub fn set_error(&self, id: &str, error: &str, now: i64) -> Result<()> {
        let c = self.conn()?;
        c.execute("INSERT OR IGNORE INTO session (session_id) VALUES (?1)", [id])?;
        c.execute("UPDATE session SET error = ?2, failures = failures + 1, updated_at = ?3 WHERE session_id = ?1", params![id, error, now])?;
        Ok(())
    }

    /// The indexed turn of a session at Funes `seq`.
    pub fn turn(&self, id: &str, fseq: i64) -> Result<Option<TurnRow>> {
        let row = self.conn()?.query_row("SELECT fseq, lp_seq, lp_end, ts FROM turn WHERE session_id = ?1 AND fseq = ?2", params![id, fseq], |r| {
            Ok(TurnRow { fseq: r.get(0)?, lp_seq: r.get(1)?, lp_end: r.get(2)?, ts: r.get(3)? })
        });
        Ok(row.optional()?)
    }

    pub fn totals(&self) -> Result<Totals> {
        let c = self.conn()?;
        let mut t = c.query_row(
            "SELECT COALESCE(SUM(turns > 0), 0), COALESCE(SUM(turns), 0), COALESCE(SUM(blocks), 0), COALESCE(SUM(truncated), 0), COALESCE(SUM(error IS NOT NULL), 0) FROM session",
            [],
            |r| Ok(Totals { sessions: r.get(0)?, turns: r.get(1)?, blocks: r.get(2)?, truncated_blocks: r.get(3)?, failed: r.get(4)?, ..Default::default() }),
        )?;
        t.newest_turn = c.query_row("SELECT COALESCE(MAX(ts), 0) FROM turn", [], |r| r.get(0))?;
        let mut st = c.prepare("SELECT skip, COUNT(*) FROM session WHERE skip IS NOT NULL GROUP BY skip")?;
        t.skipped = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?.collect::<rusqlite::Result<_>>()?;
        Ok(t)
    }

    pub fn failures(&self, limit: usize) -> Result<Vec<Failure>> {
        let c = self.conn()?;
        let mut st = c.prepare("SELECT session_id, error, failures FROM session WHERE error IS NOT NULL ORDER BY updated_at DESC LIMIT ?1")?;
        let rows = st.query_map([limit as i64], |r| Ok(Failure { session_id: r.get(0)?, error: r.get(1)?, failures: r.get(2)? }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lp-recall-ledger-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn commits_move_the_cursor_and_keep_the_funes_counter_dense() {
        let dir = temp();
        let l = Ledger::open(&dir).unwrap();
        assert_eq!(l.session("s").unwrap(), SessionRow::default());
        let first = [TurnRow { fseq: 0, lp_seq: 1, lp_end: 1, ts: 10 }, TurnRow { fseq: 1, lp_seq: 2, lp_end: 6, ts: 20 }];
        l.commit("s", 7, &first, 5, 1, 100).unwrap();
        // The same commit again (a pass interrupted after Funes, before the ledger, then run again) changes no turn.
        l.commit("s", 7, &[], 0, 0, 101).unwrap();
        let row = l.session("s").unwrap();
        assert_eq!((row.cursor, row.next_fseq, row.turns, row.skip.clone()), (7, 2, 2, None));
        l.commit("s", 12, &[TurnRow { fseq: 2, lp_seq: 8, lp_end: 11, ts: 30 }], 2, 0, 102).unwrap();
        assert_eq!(l.session("s").unwrap().next_fseq, 3);
        assert_eq!(l.turn("s", 1).unwrap().map(|t| t.lp_seq), Some(2));
        assert_eq!(l.turn("s", 9).unwrap(), None);

        // A session without any turn is counted as empty; a failure and a skip show in the totals.
        l.commit("e", 4, &[], 0, 0, 103).unwrap();
        l.set_error("f", "funes refused the file", 104).unwrap();
        l.set_skip("a", Some("archived"), 105).unwrap();
        let t = l.totals().unwrap();
        assert_eq!((t.sessions, t.turns, t.blocks, t.truncated_blocks, t.failed, t.newest_turn), (1, 3, 7, 1, 1, 30));
        assert_eq!(t.skipped, BTreeMap::from([("archived".to_string(), 1), ("empty".to_string(), 1)]));
        assert_eq!(l.failures(5).unwrap()[0].session_id, "f");
        // A later success clears the failure.
        l.commit("f", 3, &[TurnRow { fseq: 0, lp_seq: 1, lp_end: 2, ts: 40 }], 1, 0, 106).unwrap();
        assert_eq!(l.totals().unwrap().failed, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

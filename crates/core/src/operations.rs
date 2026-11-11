//! Durable claims for browser operations. A claim without a result is never executed again.
use crate::db::Db;
use anyhow::Result;
use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};

pub enum Claim {
    New,
    Completed { status: u16, body: String },
    Uncertain,
    Mismatch,
}

pub async fn claim(db: &Db, id: &str, path: &str, body: &[u8]) -> Result<Claim> {
    let id = id.to_owned();
    let path = path.to_owned();
    let hash = hex::encode(Sha256::digest(body));
    db.call(move |c| {
        let tx = c.transaction()?;
        let old = tx
            .query_row(
                "SELECT path, request_hash, status, response FROM browser_operation WHERE id=?1",
                [&id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<u16>>(2)?,
                        r.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()?;
        let result = match old {
            Some((p, h, _, _)) if p != path || h != hash => Claim::Mismatch,
            Some((_, _, Some(status), Some(body))) => Claim::Completed { status, body },
            Some(_) => Claim::Uncertain,
            None => {
                tx.execute(
                    "INSERT INTO browser_operation (id, path, request_hash) VALUES (?1, ?2, ?3)",
                    rusqlite::params![id, path, hash],
                )?;
                Claim::New
            }
        };
        tx.commit()?;
        Ok(result)
    })
    .await
}

pub async fn complete(db: &Db, id: &str, status: u16, body: String) -> Result<()> {
    let id = id.to_owned();
    db.call(move |c| {
        c.execute(
            "UPDATE browser_operation SET status=?2, response=?3, done_at=?4 WHERE id=?1",
            rusqlite::params![id, status, body, crate::util::now_ms()],
        )?;
        Ok(())
    })
    .await
}

/// How long a completed claim answers retries of its request with the saved response.
pub const KEEP_COMPLETED_MS: i64 = 7 * 24 * 3600 * 1000;

/// Deletes claims that completed more than `KEEP_COMPLETED_MS` ago. Claims without a result (in flight or uncertain) stay: they are what stops a repeat, and the owner
/// may still have to check the destination.
pub async fn prune(db: &Db, now: i64) -> Result<usize> {
    db.call(move |c| {
        Ok(c.execute(
            "DELETE FROM browser_operation WHERE status IS NOT NULL AND response IS NOT NULL AND coalesce(done_at, 0) < ?1",
            [now - KEEP_COMPLETED_MS],
        )?)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn claims_survive_reopening_and_reject_changed_requests() {
        let path =
            std::env::temp_dir().join(format!("launchpad-operations-{}.db", uuid::Uuid::new_v4()));
        let db = crate::db::open_state(&path).unwrap();
        assert!(matches!(
            claim(&db, "a", "/api/sessions", b"one").await.unwrap(),
            Claim::New
        ));
        assert!(matches!(
            claim(&db, "a", "/api/sessions", b"one").await.unwrap(),
            Claim::Uncertain
        ));
        assert!(matches!(
            claim(&db, "a", "/api/sessions", b"two").await.unwrap(),
            Claim::Mismatch
        ));
        assert!(matches!(
            claim(&db, "a", "/other", b"one").await.unwrap(),
            Claim::Mismatch
        ));
        complete(&db, "a", 200, "{\"id\":\"first\"}".into())
            .await
            .unwrap();
        let reopened = crate::db::open_state(&path).unwrap();
        match claim(&reopened, "a", "/api/sessions", b"one")
            .await
            .unwrap()
        {
            Claim::Completed { status, body } => {
                assert_eq!(status, 200);
                assert_eq!(body, "{\"id\":\"first\"}");
            }
            _ => panic!("missing saved result"),
        }
        assert!(matches!(
            claim(&db, "b", "/api/sessions/x/input", b"msg")
                .await
                .unwrap(),
            Claim::New
        ));
        assert!(matches!(
            claim(&reopened, "b", "/api/sessions/x/input", b"msg")
                .await
                .unwrap(),
            Claim::Uncertain
        ));
        // Pruning keeps the unresolved claim "b" and the fresh result of "a", then drops "a" once it is old.
        let now = crate::util::now_ms();
        assert_eq!(prune(&db, now).await.unwrap(), 0);
        assert_eq!(prune(&db, now + KEEP_COMPLETED_MS + 60_000).await.unwrap(), 1);
        assert!(matches!(claim(&db, "b", "/api/sessions/x/input", b"msg").await.unwrap(), Claim::Uncertain));
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }
}

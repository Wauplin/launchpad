//! Which sessions worked on which GitHub items (`github_ref_use` in state.db): a launch or a bot message that names an
//! issue or pull request records it, so the extension's page context finds the agents of an item and the last bot used
//! on a repository. Repository names are stored in lowercase.
use rusqlite::params;

use crate::App;
use crate::model::GitHubRef;
use crate::util::now_ms;

/// Records the items a launch or message names (best effort: a failure only logs).
pub async fn record(app: &App, session_id: &str, bot_id: Option<&str>, refs: &[GitHubRef]) {
    let rows: Vec<(String, i64)> = refs.iter().filter(|r| crate::github::valid_repo(&r.repo)).map(|r| (r.repo.to_lowercase(), r.number.unwrap_or(0) as i64)).collect();
    if rows.is_empty() {
        return;
    }
    let (session, bot, at) = (session_id.to_string(), bot_id.map(str::to_string), now_ms());
    let done = app
        .state_db
        .call(move |c| {
            for (repo, number) in rows {
                c.execute(
                    "INSERT INTO github_ref_use (repo, number, session_id, bot_id, at) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT (repo, number, session_id) DO UPDATE SET bot_id = ?4, at = ?5",
                    params![repo, number, session, bot, at],
                )?;
            }
            Ok(())
        })
        .await;
    if let Err(e) = done {
        tracing::warn!("GitHub item use of {session_id}: {e:#}");
    }
}

/// The items of `repo` sessions named (0: the repository itself) with the session, newest first.
pub async fn sessions(app: &App, repo: &str) -> anyhow::Result<Vec<(u64, String)>> {
    let repo = repo.to_lowercase();
    app.state_db
        .call(move |c| {
            let mut st = c.prepare("SELECT number, session_id FROM github_ref_use WHERE repo = ?1 ORDER BY at DESC LIMIT 500")?;
            let rows = st.query_map([repo], |r| Ok((r.get::<_, i64>(0)? as u64, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await
}

/// The bots used on `repo`, most recent first.
pub async fn bots(app: &App, repo: &str) -> anyhow::Result<Vec<String>> {
    let repo = repo.to_lowercase();
    app.state_db
        .call(move |c| {
            let mut st = c.prepare("SELECT bot_id FROM github_ref_use WHERE repo = ?1 AND bot_id IS NOT NULL GROUP BY bot_id ORDER BY max(at) DESC LIMIT 20")?;
            let rows = st.query_map([repo], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
            Ok(rows)
        })
        .await
}

/// Drops the records of sessions that ended long ago (`retention`).
pub async fn prune(app: &App, before: i64) -> anyhow::Result<usize> {
    app.state_db.call(move |c| Ok(c.execute("DELETE FROM github_ref_use WHERE at < ?1", [before])?)).await
}

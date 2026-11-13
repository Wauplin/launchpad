//! Which sessions worked on which GitHub items (`github_ref_use` in state.db): a launch or a bot message that names an
//! issue or pull request records it, so the extension's page context finds the agents of an item and the last bot used
//! on a repository. Repository names are stored in lowercase.
use rusqlite::params;

use crate::App;
use crate::model::GitHubRef;
use crate::util::now_ms;

/// Records the items a launch or message names (best effort: a failure only logs).
pub async fn record(app: &App, session_id: &str, bot_id: Option<&str>, refs: &[GitHubRef]) {
    let rows: Vec<(String, i64, Option<String>)> = refs.iter().filter(|r| crate::github::valid_repo(&r.repo)).map(|r| (r.repo.to_lowercase(), r.number.unwrap_or(0) as i64, comment_anchor(r))).collect();
    if rows.is_empty() {
        return;
    }
    let (session, bot, at) = (session_id.to_string(), bot_id.map(str::to_string), now_ms());
    let done = app
        .state_db
        .call(move |c| {
            for (repo, number, comment) in rows {
                c.execute(
                    "INSERT INTO github_ref_use (repo, number, session_id, bot_id, at, comment) VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT (repo, number, session_id) DO UPDATE SET bot_id = ?4, at = ?5, comment = ?6",
                    params![repo, number, session, bot, at, comment],
                )?;
            }
            Ok(())
        })
        .await;
    if let Err(e) = done {
        tracing::warn!("GitHub item use of {session_id}: {e:#}");
    }
}

/// The anchor of the comment a reference names (`issuecomment-123`, `discussion_r45`), from its URL's fragment.
fn comment_anchor(r: &GitHubRef) -> Option<String> {
    let (_, f) = r.url.rsplit_once('#')?;
    let ok = ["issuecomment-", "discussion_r", "pullrequestreview-"].iter().any(|p| f.strip_prefix(p).is_some_and(|n| !n.is_empty() && n.len() <= 20 && n.bytes().all(|b| b.is_ascii_digit())));
    ok.then(|| f.to_string())
}

/// The items of `repo` sessions named (0: the repository itself) with the session, newest first.
pub async fn sessions(app: &App, repo: &str) -> anyhow::Result<Vec<(u64, String)>> {
    Ok(uses(app, repo).await?.into_iter().map(|u| (u.number, u.session_id)).collect())
}

/// One record: a session named item `number` of a repository, last about `comment` (its anchor) when it named one.
pub struct Use {
    pub number: u64,
    pub session_id: String,
    pub comment: Option<String>,
    pub at: i64,
}

/// The records of `repo`, newest first.
pub async fn uses(app: &App, repo: &str) -> anyhow::Result<Vec<Use>> {
    let repo = repo.to_lowercase();
    app.state_db
        .call(move |c| {
            let mut st = c.prepare("SELECT number, session_id, comment, at FROM github_ref_use WHERE repo = ?1 ORDER BY at DESC LIMIT 500")?;
            let rows = st.query_map([repo], |r| Ok(Use { number: r.get::<_, i64>(0)? as u64, session_id: r.get(1)?, comment: r.get(2)?, at: r.get(3)? }))?.collect::<rusqlite::Result<Vec<_>>>()?;
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

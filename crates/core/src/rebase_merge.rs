//! Home: the owner's Merge of a branch whose base moved. Instead of stopping, Home asks the branch's agent to
//! rebase it (a message, placed like the owner's), then merges the rebased branch from Home Git once the agent's
//! turn ended Done and its work is on Home (`after_turn`, from the session replica). The owner's click covers that
//! merge: it never needs the launch's When done = Merge. In memory: a Home restart drops it (the owner merges
//! again), and a turn that ends another way, a question left open aside, stops it with a notice.
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use anyhow::{Context, Result, bail};

use crate::App;
use crate::home_git::MergeOutcome;
use crate::model::{Session, SessionStatus};
use crate::util::now_ms;
use crate::worktrees::{MergePhase, MergeRequest, TaskWorktree};

/// Rebases asked for one owner's Merge (the base may move again meanwhile).
const MAX_REBASES: u32 = 4;
/// A request older than this is dropped (the agent never ended its turn).
const KEEP_MS: i64 = 24 * 3_600_000;

#[derive(Clone, Debug)]
struct Pending {
    session_id: String,
    project_id: String,
    branch: String,
    base: String,
    /// Home's tip of the branch when the rebase was asked for: a turn that leaves it there did not rebase.
    from: String,
    /// Rebases asked for so far.
    rebases: u32,
    at: i64,
}

/// By worktree ID.
static PENDING: LazyLock<Mutex<HashMap<String, Pending>>> = LazyLock::new(Default::default);

/// Asks the worktree's agent to rebase `branch` (Home's tip `from`) onto its base, then merges after its turn.
/// Idempotent for the same tip; a new tip (the base moved again after a rebase) counts one more rebase.
pub async fn start(app: &App, session_id: &str, w: &TaskWorktree, from: &str, reason: &str) -> Result<()> {
    let base = w.base_branch.clone().context("this worktree has no base branch to merge into")?;
    ask(app, &w.id, Pending { session_id: session_id.into(), project_id: w.project_id.clone(), branch: w.branch.clone(), base, from: from.into(), rebases: 0, at: 0 }, reason).await
}

/// `start` for the worktree `id`; `p` gives the session, folder, branches and tip.
async fn ask(app: &App, id: &str, p: Pending, reason: &str) -> Result<()> {
    let (from, base) = (p.from.clone(), p.base.clone());
    let rebases = {
        let mut all = PENDING.lock().unwrap();
        all.retain(|_, p| now_ms() - p.at < KEEP_MS);
        let before = all.get(id).cloned();
        if before.as_ref().is_some_and(|b| b.from == from) {
            return Ok(());
        }
        let rebases = before.map(|b| b.rebases).unwrap_or(0) + 1;
        if rebases > MAX_REBASES {
            all.remove(id);
            bail!("{reason}, again after {MAX_REBASES} rebases by the agent");
        }
        all.insert(id.into(), Pending { rebases, at: now_ms(), ..p.clone() });
        rebases
    };
    let text = format!(
        "The owner asked Launchpad to merge branch `{branch}` into `{base}`, but {reason}.\nIn your worktree of `{branch}`, run `git fetch origin`, rebase onto `origin/{base}`, resolve the conflicts, re-run the relevant checks, and commit. Then end your turn: Launchpad merges the rebased branch into `{base}` once your turn is saved. Do not run `work finish` and do not merge into `{base}` yourself. If the rebase cannot be done safely, leave the branch unchanged and explain why.\nWhen you finish this turn, make your final message a concise, self-contained summary of the original task and this rebase: lead with the overall outcome, preserve the substantive results from your previous task summary, and update the checks actually performed and their results, remaining work or limitations. Report the merge as requested, not completed. The user sees this as the run's final message.",
        branch = p.branch,
    );
    if let Err(e) = crate::runners::send_anywhere(app, &p.session_id, &text, vec![], None).await {
        let mut all = PENDING.lock().unwrap();
        if all.get(id).is_some_and(|b| b.from == from && b.rebases == rebases) {
            all.remove(id);
        }
        return Err(e).context("the agent could not be asked to rebase it");
    }
    Ok(())
}

/// The merge request a worktree list shows while its agent rebases for the owner's Merge.
pub fn waiting(worktree_id: &str) -> Option<MergeRequest> {
    let p = PENDING.lock().unwrap().get(worktree_id).cloned()?;
    Some(MergeRequest {
        phase: MergePhase::Waiting,
        reason: format!("The agent rebases {} onto {}; Launchpad merges once its turn is saved", p.branch, p.base),
        by_agent: false,
        head: None,
        agent_retries: p.rebases,
        requested_at: p.at,
    })
}

/// The owner's Cancel merge.
pub fn cancel(worktree_id: &str) -> bool {
    PENDING.lock().unwrap().remove(worktree_id).is_some()
}

/// Home: a session's turn ended and its work is on Home (no `unsaved` mark). Merges the branches its agent
/// rebased for the owner.
pub fn after_turn(app: &App, s: &Session) {
    if s.status.is_running() || s.unsaved.is_some() {
        return;
    }
    let mine: Vec<(String, Pending)> = PENDING.lock().unwrap().iter().filter(|(_, p)| p.session_id == s.id).map(|(k, p)| (k.clone(), p.clone())).collect();
    for (id, p) in mine {
        let (app, s) = (app.clone(), s.clone());
        tokio::spawn(async move {
            if let Some(text) = merge_after(&app, &s, &id, &p).await {
                crate::sessions::notice_anywhere(&app, &s.id, &text).await;
            }
        });
    }
}

/// The merge after the agent's turn; returns the notice for the session.
async fn merge_after(app: &App, s: &Session, id: &str, p: &Pending) -> Option<String> {
    let still = |p: &Pending| PENDING.lock().unwrap().get(id).is_some_and(|now| now.from == p.from && now.rebases == p.rebases);
    if !still(p) || s.open_question || s.status == SessionStatus::QuotaPaused {
        return None;
    }
    // An older record of the session (its turn before the message) changes nothing.
    let tip = crate::home_git::tip(app, &p.project_id, &format!("refs/heads/{}", p.branch)).await.ok().flatten();
    if tip.as_deref() == Some(p.from.as_str()) && s.status_since < p.at {
        return None;
    }
    let stop = |why: String| {
        PENDING.lock().unwrap().remove(id);
        Some(format!("Merge of {} into {} stopped: {why}. Merge again when it is ready.", p.branch, p.base))
    };
    if s.status != SessionStatus::Waiting {
        return stop(format!("the agent's turn ended {}", s.status.as_str()));
    }
    let Some(tip) = tip else { return stop(format!("{} is no longer on Home", p.branch)) };
    if tip == p.from {
        return stop("the agent left the branch unchanged".into());
    }
    let project = match crate::projects::get(app, &p.project_id).await {
        Ok(Some(project)) => project,
        _ => return stop("the folder no longer exists".into()),
    };
    match crate::worktrees::merge_on_home(app, &project, &p.branch, &tip, &p.base).await {
        Ok(MergeOutcome::Merged { head, note }) => {
            PENDING.lock().unwrap().remove(id);
            Some(format!("Launchpad merged {} into {} ({}) after the agent's rebase. {note}", p.branch, p.base, &head[..12.min(head.len())]))
        }
        Ok(MergeOutcome::NeedsRebase { reason, .. }) => {
            match ask(app, id, Pending { from: tip, ..p.clone() }, &reason).await {
                Ok(()) => None,
                Err(e) => stop(format!("{e:#}")),
            }
        }
        Err(e) => stop(format!("{e:#}")),
    }
}

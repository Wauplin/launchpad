//! Home: When done = Draft PR or Open PR. The agent's `work finish` opens a Local PR of its branch on Home; once the
//! turn ended Done and its work is on Home (`after_turn`, from the session replica, like `rebase_merge`), Home
//! publishes that Local PR on GitHub with the owner's login, as `predrafts::publish` does for the owner's Publish.
//! Only the owner's own launches (`LaunchSpec::owner_launch`): choosing it in the composer is the owner's click.
//! A Local PR is published at most once per Home run, and only one whose publication never started (no claimed
//! head or remote branch), so an uncertain publication is never retried: it waits for the owner's Publish or Unlock.
use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

use crate::App;
use crate::model::{Session, SessionStatus};
use crate::predrafts::{Predraft, Publish, PublishSet};

/// Local PRs whose publication this Home run already started, by ID.
static STARTED: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Default::default);

/// Home: a session's turn ended and its work is on Home (no `unsaved` mark).
pub fn after_turn(app: &App, s: &Session) {
    if s.status != SessionStatus::Waiting || s.unsaved.is_some() || s.open_question || !s.launch_spec.owner_launch() {
        return;
    }
    let Some(draft) = s.launch_spec.when_done().publishes() else { return };
    let (app, s) = (app.clone(), s.clone());
    tokio::spawn(async move {
        for text in publish(&app, &s, draft).await {
            crate::sessions::notice_anywhere(&app, &s.id, &text).await;
        }
    });
}

/// Publishes the session's Local PRs that never started a publication; returns the notices for the session.
async fn publish(app: &App, s: &Session, draft: bool) -> Vec<String> {
    let all = match crate::predrafts::list(app).await {
        Ok(all) => all,
        Err(e) => return vec![format!("Pull request not opened: Home could not read its Local PRs ({e:#}). Publish it from its Local PR page.")],
    };
    let unstarted = |p: &Predraft| p.is_open() && p.pushed_head.is_none() && p.remote_branch.is_none();
    let mine: Vec<Predraft> = {
        let mut started = STARTED.lock().unwrap();
        all.iter().filter(|p| p.session_id.as_deref() == Some(s.id.as_str()) && unstarted(p) && started.insert(p.id.clone())).cloned().collect()
    };
    let kind = if draft { "draft pull request" } else { "pull request" };
    let mut notices = vec![];
    let mut done: HashSet<String> = HashSet::new();
    for p in &mine {
        if done.contains(&p.id) {
            continue;
        }
        // Several Local PRs of one change set are published together and linked to each other.
        let set: Vec<&Predraft> = all.iter().filter(|o| p.change_set.is_some() && o.change_set == p.change_set && o.published_url.is_none() && !o.archived && o.merged.is_none()).collect();
        let result = if set.iter().any(|m| !unstarted(m)) {
            set.iter().for_each(|m| { done.insert(m.id.clone()); });
            Err(anyhow::anyhow!("another Local PR of its change set has a publication that did not finish"))
        } else if set.len() > 1 {
            let mut heads = std::collections::HashMap::new();
            for m in &set {
                if let Ok(Some(head)) = crate::home_git::tip(app, &m.project_id, &format!("refs/heads/{}", m.branch)).await {
                    heads.insert(m.id.clone(), head);
                }
                done.insert(m.id.clone());
            }
            crate::predrafts::publish_set(app, &p.id, PublishSet { draft, heads }).await.map(|v| v.into_iter().filter_map(|p| p.published_url).collect::<Vec<_>>())
        } else {
            done.insert(p.id.clone());
            match crate::home_git::tip(app, &p.project_id, &format!("refs/heads/{}", p.branch)).await {
                Ok(Some(head)) => {
                    let remote_branch = p.default_remote_branch.clone();
                    crate::predrafts::publish(app, &p.id, Publish { remote_branch, draft, head, fork: false }).await.map(|p| p.published_url.into_iter().collect())
                }
                Ok(None) => Err(anyhow::anyhow!("{} is not on Home", p.branch)),
                Err(e) => Err(e),
            }
        };
        notices.push(match result {
            Ok(urls) if !urls.is_empty() => format!("Launchpad opened the {kind} {} on GitHub (When done).", urls.join(", ")),
            Ok(_) => format!("Local PR \"{}\" was not published on GitHub. Publish it from its Local PR page.", p.title),
            Err(e) => format!("Launchpad could not open the {kind} of Local PR \"{}\": {e:#}. Publish it from its Local PR page.", p.title),
        });
    }
    notices
}

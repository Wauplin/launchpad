//! This crate's part of Today (`launchpad_core::today`): proposal cards that wait or were handled, and routines
//! whose last run failed (until they run again or are paused).
use launchpad_core::App;
use launchpad_core::today::{Filter, Item, Kind, Place, line};

use crate::crons::{self, RunOutcome};
use crate::proposals::{self, Proposal, ProposalBody, ProposalState, ReportOutcome};

/// A proposal's workspace; without one, the workspace of what it acts on. Tools and presets are shared.
fn place(p: &Proposal) -> Place {
    if p.workspace_id.is_some() {
        return Place::Workspace(p.workspace_id.clone());
    }
    match &p.body {
        ProposalBody::Launch { spec } | ProposalBody::Schedule { spec, .. } | ProposalBody::Cron { spec, .. } => Place::of_spec(spec),
        ProposalBody::Message { session_id, .. } => Place::Session(session_id.clone()),
        ProposalBody::Status { session_ids } => session_ids.first().cloned().map(Place::Session).unwrap_or(Place::Workspace(None)),
        ProposalBody::Tool { .. } | ProposalBody::Preset { .. } => Place::Everywhere,
        _ => p.session_id.clone().map(Place::Session).unwrap_or(Place::Workspace(None)),
    }
}

pub async fn items(app: &App) -> Vec<Item> {
    let mut out = vec![];
    match proposals::list(app).await {
        Ok(list) => out.extend(list.into_iter().filter_map(|p| {
            let place = place(&p);
            // A report waits under Answer when it asks something, else under Review; it links its GitHub item.
            let (filter, github) = match &p.body {
                ProposalBody::Report { outcome, github, .. } => (if *outcome == ReportOutcome::NeedsYou { Filter::Answer } else { Filter::Review }, github.clone()),
                _ => (Filter::Approve, None),
            };
            let card = |at: i64| {
                let item = Item::new(Kind::Proposal, filter, &p.id, &p.title, place.clone(), at).session(p.session_id.clone());
                Item { github: github.clone(), ..item }
            };
            let report = match &p.body { ProposalBody::Report { outcome, .. } => Some(*outcome), _ => None };
            // A ready card's main action is the owner action it names; a question is answered, other reports read.
            let main = match &p.body {
                ProposalBody::Ready { action, .. } => launchpad_core::actions::get(action).map(|a| a.name),
                ProposalBody::Status { .. } => Some("proposal.dismiss"),
                ProposalBody::Report { outcome: ReportOutcome::NeedsYou, .. } => Some("proposal.reply"),
                ProposalBody::Report { .. } => Some("proposal.dismiss"),
                _ => None,
            };
            let detail = match &p.body { ProposalBody::Report { body, .. } if p.note.is_none() => line(body), _ => p.note.as_deref().map(line).unwrap_or_default() };
            Some(match p.state {
                ProposalState::Proposed => card(p.created_at).detail(detail).action(main.unwrap_or("proposal.start")),
                ProposalState::Failed if report.is_some() => card(p.updated_at).detail(format!("Reply failed: {}", p.error.as_deref().map(line).unwrap_or_default())).action("proposal.reply"),
                ProposalState::Failed => card(p.updated_at).detail(format!("Failed: {}", p.error.as_deref().map(line).unwrap_or_default())).action("proposal.start"),
                // Never retried: the owner checks first.
                ProposalState::Uncertain => card(p.updated_at).detail("Uncertain: check whether it started before you dismiss it").action("proposal.dismiss"),
                ProposalState::Started => card(p.updated_at).handled(if report.is_some() { "Answered" } else { "Started" }),
                ProposalState::Dismissed => card(p.updated_at).handled(if report.is_some() { "Read" } else { "Dismissed" }),
                ProposalState::Starting | ProposalState::Replaced => return None,
            })
        })),
        Err(e) => tracing::warn!("Today: proposals: {e:#}"),
    }
    match crons::list(app).await {
        Ok(list) => out.extend(list.into_iter().filter_map(|c| {
            // A paused routine waits for nobody.
            if c.paused || c.bot_paused {
                return None;
            }
            // Gate checks are not runs: the last run decides.
            let last = c.history.iter().rev().find(|x| x.outcome != RunOutcome::Checked)?;
            let what = match last.outcome {
                RunOutcome::Failed => "failed",
                RunOutcome::Interrupted => "was interrupted",
                // Missed: it waited an hour for confirmed quota, or an event found none.
                RunOutcome::Skipped if last.missed => "was skipped",
                _ => return None,
            };
            let detail = last.detail.as_deref().map(line).unwrap_or_default();
            Some(Item::new(Kind::Routine, Filter::Fix, &c.id, format!("Routine “{}” {what}", c.title), Place::of_spec(&c.spec), last.at).detail(detail).session(last.session_id.clone()))
        })),
        Err(e) => tracing::warn!("Today: routines: {e:#}"),
    }
    if launchpad_core::home_git::here(app) {
        match oracle_items(app).await {
            Ok(items) => out.extend(items),
            Err(e) => tracing::warn!("Today: oracles: {e:#}"),
        }
    }
    out
}

/// One Fix row per failing oracle (its open failure streak), updated in place: the count, the last error, its
/// dominant class and the routines gated on it. A recovery or Mark read moves it to Done until the next failure.
async fn oracle_items(app: &App) -> anyhow::Result<Vec<Item>> {
    let mut out = vec![];
    for s in crate::oracle_eval::streaks(app, None).await? {
        let recovered = s.count == 0 && s.recovered_at.is_some();
        if !s.open() && !recovered && s.read_at.is_none() {
            continue;
        }
        let title = crate::oracles::get(app, &s.workspace_id, &s.oracle).await.ok().and_then(|f| f.oracle.meta).map(|m| m.title).unwrap_or_else(|| s.oracle.clone());
        let place = Place::Workspace(Some(s.workspace_id.clone()));
        let item = |at: i64, what: String| {
            let mut i = Item::new(Kind::Oracle, Filter::Fix, &s.oracle, what, place.clone(), at);
            i.id = format!("oracle:{}/{}", s.workspace_id, s.oracle);
            i
        };
        if s.open() {
            let users = crate::gates::users(app, &s.workspace_id, &s.oracle).await.unwrap_or_default();
            let mut detail = match (&s.last_error, s.dominant()) {
                (Some(e), Some(d)) if d != e.class => format!("{}: {} (mostly {})", e.class.as_str(), line(&e.message), d.as_str()),
                (Some(e), _) => format!("{}: {}", e.class.as_str(), line(&e.message)),
                (None, _) => "failed".into(),
            };
            match users.as_slice() {
                [] => {}
                [one] => detail.push_str(&format!(" · waiting: routine “{}”", one.title)),
                many => detail.push_str(&format!(" · waiting: {} routines", many.len())),
            }
            let n = s.count;
            let detail = format!("{n} failed evaluation{} · {detail}", if n == 1 { "" } else { "s" });
            let mut i = item(s.last_at.unwrap_or(0), format!("Oracle “{title}” is failing")).detail(detail);
            i.count = Some(s.count);
            out.push(i);
        } else if s.count > 0 {
            // Marked read: Done until the next failure starts a new streak.
            out.push(item(s.read_at.unwrap_or(0), format!("Oracle “{title}” is failing")).handled("Marked read"));
        } else if let Some(at) = s.recovered_at {
            let n = s.recovered_after;
            out.push(item(at, format!("Oracle “{title}” recovered")).handled(format!("Recovered after {n} failed evaluation{}", if n == 1 { "" } else { "s" })));
        }
    }
    Ok(out)
}

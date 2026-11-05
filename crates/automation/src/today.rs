//! This crate's part of Today (`launchpad_core::today`): proposal cards that wait or were handled, and routines
//! whose last run failed (until they run again or are paused).
use launchpad_core::App;
use launchpad_core::today::{Filter, Item, Kind, Place, line};

use crate::crons::{self, RunOutcome};
use crate::proposals::{self, Proposal, ProposalBody, ProposalState};

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
            let card = |at: i64| Item::new(Kind::Proposal, Filter::Approve, &p.id, &p.title, place.clone(), at).session(p.session_id.clone());
            let status = matches!(p.body, ProposalBody::Status { .. });
            // A ready card's main action is the owner action it names.
            let main = match &p.body { ProposalBody::Ready { action, .. } => launchpad_core::actions::get(action).map(|a| a.name), _ => None };
            Some(match p.state {
                ProposalState::Proposed => card(p.created_at).detail(p.note.as_deref().map(line).unwrap_or_default()).action(main.unwrap_or(if status { "proposal.dismiss" } else { "proposal.start" })),
                ProposalState::Failed => card(p.updated_at).detail(format!("Failed: {}", p.error.as_deref().map(line).unwrap_or_default())).action("proposal.start"),
                // Never retried: the owner checks first.
                ProposalState::Uncertain => card(p.updated_at).detail("Uncertain: check whether it started before you dismiss it").action("proposal.dismiss"),
                ProposalState::Started => card(p.updated_at).handled("Started"),
                ProposalState::Dismissed => card(p.updated_at).handled("Dismissed"),
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
            let last = c.history.last()?;
            let what = match last.outcome {
                RunOutcome::Failed => "failed",
                RunOutcome::Interrupted => "was interrupted",
                _ => return None,
            };
            let detail = last.detail.as_deref().map(line).unwrap_or_default();
            Some(Item::new(Kind::Routine, Filter::Fix, &c.id, format!("Routine “{}” {what}", c.title), Place::of_spec(&c.spec), last.at).detail(detail).session(last.session_id.clone()))
        })),
        Err(e) => tracing::warn!("Today: routines: {e:#}"),
    }
    out
}

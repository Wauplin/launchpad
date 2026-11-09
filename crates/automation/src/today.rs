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

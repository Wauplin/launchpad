//! This crate's part of Today (`launchpad_core::today`): GitHub suggestions to post, open pull requests that request
//! your review, and posted suggestions (Done). Notes on Local PRs belong to the Local PR's card.
use std::collections::HashSet;

use launchpad_core::App;
use launchpad_core::today::{Filter, Item, Kind, Place, line, parse_time};

use crate::suggestions::{self, SuggestionKind, SuggestionTarget};

pub async fn items(app: &App) -> Vec<Item> {
    let mut out = vec![];
    // Pull requests that already have a suggested review: its card stands for the review request.
    let mut reviewed: HashSet<String> = HashSet::new();
    match suggestions::list(app).await {
        Ok(list) => {
            for s in list {
                let SuggestionTarget::GitHub { repo, number, reply_to } = &s.target else { continue };
                let what = match (&s.kind, &s.merge, s.run_ci) {
                    (SuggestionKind::Review, Some(_), _) => "Approve and merge",
                    (SuggestionKind::Review, None, true) => "Review and run CI",
                    (SuggestionKind::Review, None, false) => "Review",
                    _ if reply_to.is_some() => "Reply",
                    _ => "Comment",
                };
                let item = Item::new(Kind::Suggestion, Filter::Github, &s.id, format!("{what} on {repo}#{number}"), Place::Repo(repo.clone()), s.created_at)
                    .session(s.session_id.clone())
                    .github(repo, number)
                    .detail(match s.comments.len() {
                        0 => format!("Suggestion by {} · {}", s.author, line(&s.body)),
                        1 => format!("Suggestion by {} · 1 line comment · {}", s.author, line(&s.body)),
                        n => format!("Suggestion by {} · {n} line comments · {}", s.author, line(&s.body)),
                    });
                out.push(match s.status.as_str() {
                    "posted" => item.handled(match &s.followup_error {
                        Some(e) => format!("Posted to {repo}#{number}; {}", line(e)),
                        None => format!("Posted to {repo}#{number}"),
                    }),
                    "inserted" => item.handled(format!("Inserted on GitHub, in {repo}#{number}'s comment box")),
                    "posted_by_hand" => item.handled(format!("Posted by you on {repo}#{number}")),
                    "posting" | "uncertain" => item.detail("Uncertain: check GitHub before you discard it").action("suggestion.discard"),
                    _ => {
                        if s.kind == SuggestionKind::Review {
                            reviewed.insert(format!("{}#{number}", repo.to_lowercase()));
                        }
                        item.action("suggestion.post")
                    }
                });
            }
        }
        Err(e) => tracing::warn!("Today: suggestions: {e:#}"),
    }
    match crate::inbox::of(app).list(app).await {
        Ok(snapshot) => out.extend(snapshot.threads.into_iter().filter_map(|t| {
            let number = t.number?;
            let open = t.reason == "review_requested" && t.kind == "PullRequest" && !matches!(t.state.as_deref(), Some("closed" | "merged"));
            if !open || reviewed.contains(&format!("{}#{number}", t.repo.to_lowercase())) {
                return None;
            }
            Some(Item::new(Kind::ReviewRequest, Filter::Github, &t.id, &t.title, Place::Repo(t.repo.clone()), parse_time(&t.updated_at)).detail(format!("{}#{number} · review requested", t.repo)).github(&t.repo, number))
        })),
        Err(e) => tracing::warn!("Today: review requests: {e:#}"),
    }
    out
}

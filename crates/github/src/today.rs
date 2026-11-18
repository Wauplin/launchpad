//! This crate's part of Today (`launchpad_core::today`): GitHub suggestions to post and posted suggestions (Done).
//! Notes on Local PRs belong to the Local PR's card.
use launchpad_core::App;
use launchpad_core::today::{Filter, Item, Kind, Place, line};

use crate::suggestions::{self, SuggestionKind, SuggestionTarget};

pub async fn items(app: &App) -> Vec<Item> {
    let mut out = vec![];
    match suggestions::list(app).await {
        Ok(list) => {
            for s in list {
                let SuggestionTarget::GitHub { repo, number, .. } = &s.target else { continue };
                let what = suggestions::headline(&s);
                let item = Item::new(Kind::Suggestion, Filter::Github, &s.id, format!("{what} on {repo}#{number}"), Place::Repo(repo.clone()), s.created_at)
                    .session(s.session_id.clone())
                    .github(repo, number)
                    .detail(match s.comments.len() {
                        _ if s.kind == SuggestionKind::Action => format!("Suggestion by {} · {}", s.author, s.actions.iter().map(|a| a.describe()).collect::<Vec<_>>().join(", ")),
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
                    _ => item.action("suggestion.post"),
                });
            }
        }
        Err(e) => tracing::warn!("Today: suggestions: {e:#}"),
    }
    out
}

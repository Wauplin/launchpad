// The actions a suggestion also runs on its GitHub issue or pull request when the owner posts it (`ItemAction`,
// crates/github/src/suggestions.rs): after its comment or review, or alone for an action-only suggestion. Shared by
// the app's suggestion card and the Chrome extension's timeline. Keep the words in step with `ItemAction::describe`
// and `headline` in Rust.

import type { ItemAction } from "@/gen/ItemAction";

/** What an action does, in words ("close as not planned", "request reviews from @ana, team core"). */
export function describeAction(a: ItemAction): string {
  switch (a.kind) {
    case "close":
      return a.reason === "completed" ? "close as completed" : a.reason === "not_planned" ? "close as not planned" : a.reason === "duplicate" ? `close as duplicate of #${a.duplicate_of ?? "?"}` : "close";
    case "reopen": return "reopen";
    case "request_reviewers": return `request reviews from ${[...a.reviewers.map((r) => `@${r}`), ...a.team_reviewers.map((t) => `team ${t}`)].join(", ")}`;
    case "ready_for_review": return "mark ready for review";
    case "convert_to_draft": return "convert to draft";
    case "update_branch": return "update the branch";
  }
}

const RANK: Record<ItemAction["kind"], number> = { reopen: 0, update_branch: 1, convert_to_draft: 2, ready_for_review: 2, request_reviewers: 3, close: 4 };

/** The actions in the order Post runs them (reopen first, close last). */
export const ranked = (list: ItemAction[]): ItemAction[] => [...list].sort((a, b) => RANK[a.kind] - RANK[b.kind]);

export const capitalized = (s: string): string => s.charAt(0).toUpperCase() + s.slice(1);

/** "a, b and c". */
export function joined(parts: string[]): string {
  return parts.length < 2 ? parts.join("") : `${parts.slice(0, -1).join(", ")} and ${parts[parts.length - 1]}`;
}

/** The button of a post: what the click does on GitHub ("Comment and close as not planned", "Request reviews"). `base`
 * names the text part ("Comment", "Approve and merge (squash)"), empty for an action-only suggestion. */
export function postButton(base: string, actions: ItemAction[]): string {
  const parts = ranked(actions).map((a) => a.kind === "request_reviewers" ? "request reviews" : describeAction(a));
  return capitalized(joined(base ? [base, ...parts] : parts)) || "Post";
}

/** Pure: the reviewers the owner typed (`ana, @bo, org/core`): logins and team slugs. */
export function parseReviewers(text: string): { reviewers: string[]; team_reviewers: string[] } {
  const out = { reviewers: [] as string[], team_reviewers: [] as string[] };
  for (const raw of text.split(/[\s,]+/).map((r) => r.replace(/^@/, "")).filter(Boolean)) {
    const slash = raw.indexOf("/");
    if (slash >= 0) out.team_reviewers.push(raw.slice(slash + 1));
    else out.reviewers.push(raw);
  }
  return out;
}

/** The reviewers as the owner edits them. */
export const reviewersText = (a: Extract<ItemAction, { kind: "request_reviewers" }>): string => [...a.reviewers, ...a.team_reviewers.map((t) => `team/${t}`)].join(", ");

/** A stable key of an action, to tick it on or off. */
export const actionKey = (a: ItemAction): string => a.kind;

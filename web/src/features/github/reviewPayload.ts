import type { PendingReview, ReviewPayload } from "./pendingReview";
import type { ReviewEvent } from "./ReplyBox";

export function reviewPayload(review: PendingReview, head: string, event: ReviewEvent): ReviewPayload {
  return { source: review.source, merge: event === "APPROVE" && review.merge, run_ci: review.runCi, body: review.summary, event, commit_id: head, comments: review.comments.filter((n) => !n.head || n.head === head).map((n) => ({ path: n.path, line: n.line, side: n.side === "old" ? "LEFT" : "RIGHT", body: n.text })) };
}

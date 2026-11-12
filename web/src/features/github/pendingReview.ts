import { useEffect } from "react";
import { useReviewState } from "@/lib/reviewState";
import type { LineNote } from "./DiffReview";
import type { ReviewEvent } from "./ReplyBox";

export type ReviewPayload = { body: string; event: ReviewEvent; commit_id: string; comments: { path: string; line: number | null; side: string; body: string; start_line?: number }[]; source: string | null; merge: boolean; run_ci: boolean };
export type PendingReview = { summary: string; event: ReviewEvent | null; merge: boolean; runCi: boolean; comments: LineNote[]; source: string | null; head: string | null; attempt: ReviewPayload | null };
export const emptyReview: PendingReview = { summary: "", event: null, merge: false, runCi: false, comments: [], source: null, head: null, attempt: null };
export const pendingReviewKey = (repo: string, number: number) => `pending-review:${repo.toLowerCase()}#${number}`;
export function usePendingReview(repo: string, number: number) {
  const draft = useReviewState(pendingReviewKey(repo, number), emptyReview);
  const earlier = useReviewState<LineNote[]>(`pr-notes:${repo}#${number}`, []);
  // Keep line comments written before the combined panel was introduced.
  useEffect(() => {
    if (!draft.loaded || !earlier.loaded || !earlier.value.length) return;
    draft.set((v) => ({ ...v, comments: [...v.comments, ...earlier.value.filter((n) => !v.comments.some((old) => JSON.stringify(old) === JSON.stringify(n)))] }));
    void earlier.clear();
  }, [draft.loaded, earlier.loaded, earlier.value]);
  return draft;
}

// One reply box for GitHub issues and PRs, used by the Inbox preview and the item page. The unsent reply lives in
// review_state (`reply:<repo>#<n>`), so both places and every device share it. Posting needs an explicit click.
// A lost response is never retried automatically: the owner checks GitHub, then explicitly starts a new attempt.

import { useEffect, useRef, useState, type ReactNode } from "react";
import { toast } from "@/components/Toast";
import { ConfirmButton, ErrorState, FilterChip, FilterChips } from "@/components/ui";
import { api } from "@/lib/api";
import { abandonOperation } from "@/lib/operations";
import { replyKey, useReviewState } from "@/lib/reviewState";

export type ReviewEvent = "COMMENT" | "APPROVE" | "REQUEST_CHANGES";
type Attempt = { path: string; body: unknown };

/**
 * `comment`: endpoint and body for a plain comment. `review`: present for open PRs; `body` builds the review request
 * (line comments included by the caller). `notes`: an extra line shown above the buttons (pending line comments).
 */
export function GitHubReplyBox({ repo, number, comment, review, notes, onPosted, collapsible = false, disabled }: {
  repo: string;
  number: number;
  comment: { path: string; body: (text: string) => unknown };
  review?: { path: string; body: (text: string, event: ReviewEvent) => unknown; extra?: number };
  notes?: ReactNode;
  onPosted?: (kind: "comment" | "review") => void;
  collapsible?: boolean;
  disabled?: boolean;
}) {
  const reply = useReviewState(replyKey(repo, number), "");
  const text = reply.value;
  const [event, setEvent] = useState<ReviewEvent>("COMMENT");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [failed, setFailed] = useState<Attempt | null>(null);
  const [open, setOpen] = useState(!collapsible);
  const box = useRef<HTMLDivElement>(null);
  // WebKit does not focus clicked buttons, so the box collapses on a click outside it rather than on blur.
  useEffect(() => {
    if (!collapsible || !open) return;
    const down = (e: MouseEvent) => { if (!box.current?.contains(e.target as Node)) setOpen(false); };
    document.addEventListener("mousedown", down);
    return () => document.removeEventListener("mousedown", down);
  }, [collapsible, open]);
  const expanded = open || !!text.trim() || !!failed || !!review?.extra;
  async function send(kind: "comment" | "review") {
    const attempt: Attempt = kind === "comment" ? { path: comment.path, body: comment.body(text) } : { path: review!.path, body: review!.body(text, event) };
    setBusy(true); setError(null);
    try {
      await api.post(attempt.path, attempt.body);
      setFailed(null);
      await reply.clear();
      toast(kind === "comment" ? `Comment posted on #${number}` : `Review submitted on #${number}`);
      onPosted?.(kind);
    } catch (e) {
      setFailed(attempt);
      setError(e);
    } finally { setBusy(false); }
  }
  const canReview = !!review && (event === "APPROVE" || !!text.trim() || !!review.extra);
  return <div ref={box} className="flex-none border-t border-line bg-chrome p-3" onFocus={() => setOpen(true)}>
    {failed && <div className="mb-2 grid gap-2">
      <ErrorState error={error ?? "The last attempt has no confirmed result."} />
      <div className="flex flex-wrap items-center gap-2 text-[12px] text-ink-2">
        <span>It may have reached GitHub. Check GitHub before trying again.</span>
        <ConfirmButton tone="publish" consequence="Only if it was not posted; otherwise it is duplicated." confirmLabel="Start a new attempt" onConfirm={async () => { await abandonOperation(failed.path, failed.body); setFailed(null); setError(null); }}>I checked GitHub</ConfirmButton>
      </div>
    </div>}
    <textarea
      className={`field-input ${expanded ? "h-24 resize-y py-2" : "h-[34px] resize-none overflow-hidden"}`}
      rows={1}
      aria-label="Reply"
      placeholder="Write a reply…"
      value={text}
      disabled={busy || disabled}
      onChange={(e) => reply.set(e.target.value)}
    />
    {expanded && <div className="mt-2 flex flex-wrap items-center gap-2">
      {review && <FilterChips label="Review type">
        {([["COMMENT", "Comment"], ["APPROVE", "Approve"], ["REQUEST_CHANGES", "Request changes"]] as const).map(([id, label]) => <FilterChip key={id} active={event === id} onClick={() => setEvent(id)}>{label}</FilterChip>)}
      </FilterChips>}
      {notes}
      <span className="ml-auto text-[12px] text-ink-3">Posts to GitHub when you click.</span>
      {review && <button className="btn btn-sm" disabled={busy || disabled || !canReview} onClick={() => void send("review")}>{busy ? "Posting…" : "Submit review"}</button>}
      <button className="btn btn-sm btn-primary" disabled={busy || disabled || !text.trim()} onClick={() => void send("comment")}>{busy ? "Posting…" : "Post comment"}</button>
    </div>}
  </div>;
}

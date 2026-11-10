// One reply box for GitHub issues and PRs, used by the Inbox preview and the item page. The unsent reply lives in
// review_state (`reply:<repo>#<n>`), so both places and every device share it. Posting needs an explicit click.
// A lost response is never retried automatically: the owner checks GitHub, then explicitly starts a new attempt.

import { useEffect, useRef, useState } from "react";
import { toast } from "@/components/Toast";
import { ConfirmButton, ErrorState } from "@/components/ui";
import { api } from "@/lib/api";
import { abandonOperation } from "@/lib/operations";
import { replyKey, useReviewState } from "@/lib/reviewState";

export type ReviewEvent = "COMMENT" | "APPROVE" | "REQUEST_CHANGES";
type Attempt = { path: string; body: unknown };

export function GitHubReplyBox({ repo, number, comment, onPosted, collapsible = false, disabled }: {
  repo: string;
  number: number;
  comment: { path: string; body: (text: string) => unknown };
  onPosted?: () => void;
  collapsible?: boolean;
  disabled?: boolean;
}) {
  const reply = useReviewState(replyKey(repo, number), "");
  const text = reply.value;
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
  const expanded = open || !!text.trim() || !!failed;
  async function send() {
    const attempt: Attempt = { path: comment.path, body: comment.body(text) };
    setBusy(true); setError(null);
    try {
      await api.post(attempt.path, attempt.body);
      setFailed(null);
      await reply.clear();
      toast(`Comment posted on #${number}`);
      onPosted?.();
    } catch (e) {
      setFailed(attempt);
      setError(e);
    } finally { setBusy(false); }
  }
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
      placeholder="Write a comment"
      value={text}
      disabled={busy || disabled}
      onChange={(e) => reply.set(e.target.value)}
    />
    {expanded && <div className="mt-2 flex flex-wrap items-center gap-2">
      <span className="ml-auto text-[12px] text-ink-3">Posts to GitHub when you click.</span>
      <button className="btn btn-sm btn-primary" disabled={busy || disabled || !text.trim()} onClick={() => void send()}>{busy ? "Posting…" : "Post comment"}</button>
    </div>}
  </div>;
}

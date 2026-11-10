import type { Suggestion } from "@/gen/Suggestion";
import type { PendingReview } from "./pendingReview";
import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { ApiError, api } from "@/lib/api";
import { abandonOperation } from "@/lib/operations";
import { useReviewState } from "@/lib/reviewState";
import { ConfirmButton, ErrorState } from "@/components/ui";
import { Markdown } from "@/components/Markdown";
import { MenuItem, Popover } from "@/components/Popover";
import { useSuggestions } from "@/features/suggestions/Suggestions";
import { itemPath } from "./lib";
import { reviewPayload } from "./reviewPayload";
import { usePendingReview } from "./pendingReview";
import type { ReviewEvent } from "./ReplyBox";

export function ReviewPanel({ repo, number, head, onPosted, onComment }: { repo: string; number: number; head: string; onPosted: () => void; onComment: (path: string, line: number | null) => void }) {
  const draft = usePendingReview(repo, number);
  const preference = useReviewState<ReviewEvent>(`review-event:${repo.toLowerCase()}`, "APPROVE");
  const d = draft.value, event = d.event ?? preference.value;
  const qc = useQueryClient();
  const suggestions = useSuggestions({ kind: "git_hub", repo, number }).data.filter((s) => s.kind === "review" && s.status !== "posted");
  const [busy, setBusy] = useState(false), [error, setError] = useState<unknown>(null);
  const attempt = d.attempt;
  const labels = { APPROVE: "Approve", COMMENT: "Comment", REQUEST_CHANGES: "Request changes" };
  const current = d.comments.filter((n) => !n.head || n.head === head);
  const stale = !!d.head && d.head !== head;
  async function post() {
    const payload = reviewPayload(d, head, event);
    setBusy(true); setError(null); preference.set(event);
    draft.set((v) => ({ ...v, attempt: payload }));
    try {
      const result = await api.post<{ followup_error?: string }>(`${itemPath(repo, number)}/review`, payload);
      await draft.clear();
      void qc.invalidateQueries({ queryKey: ["suggestions"] });
      if (result.followup_error) setError(result.followup_error);
      onPosted();
    } catch (e) {
      setError(e);
      // Home answered with a refusal (PR moved, GitHub rejected it): nothing was posted, so the review stays editable.
      // Without a definite answer, the attempt stays until the owner checked GitHub.
      if (e instanceof ApiError && e.operationCompleted) draft.set((v) => ({ ...v, attempt: null }));
      void qc.invalidateQueries({ queryKey: ["suggestions"] });
    } finally { setBusy(false); }
  }
  const sourceUnavailable = !!d.source && !suggestions.some((s) => s.id === d.source && s.status === "draft");
  const disabled = busy || !!attempt || sourceUnavailable || stale || !draft.loaded || (event !== "APPROVE" && !d.summary.trim() && !current.length);
  return <section className="grid gap-3 p-3" aria-label="Your review">
    <b>Your review</b>
    <textarea aria-label="Review summary" className="field-input min-h-24" placeholder="Review summary…" disabled={busy} value={d.summary} onChange={(e) => draft.set((v) => ({ ...v, summary: e.target.value, event }))} />
    <span className="text-[12px] text-ink-3">{current.length} comments · Draft shared across devices</span>
    {d.comments.map((n, i) => <div key={i} className="grid gap-1 border-l border-line pl-2 text-[12px]">
      <button className="text-left font-mono text-teal-ink" onClick={() => onComment(n.path, n.line)}>{n.path}:{n.line}{n.head && n.head !== head ? " · older commit, excluded" : ""}</button>
      <span className="whitespace-pre-wrap">{n.text}</span>
      <button className="btn btn-sm w-fit" disabled={busy} onClick={() => draft.set((v) => ({ ...v, comments: v.comments.filter((_, j) => j !== i) }))}>Delete</button>
    </div>)}
    {suggestions.map((s) => <AgentDraft key={s.id} suggestion={s} head={head} busy={busy || !!attempt} selected={d.source === s.id} onUse={(v) => draft.set((old) => ({ ...old, ...v, source: s.id, head: s.head }))} onDrop={async () => { await api.delete(`/api/suggestions/${s.id}`); if (d.source === s.id) draft.set((v) => ({ ...v, source: null, head: null })); await qc.invalidateQueries({ queryKey: ["suggestions"] }); }} onError={setError} />)}
    {sourceUnavailable && <span className="text-attn text-[12px]">The source suggestion was posted or dropped. Check GitHub before continuing.</span>}
    {stale && <span className="text-attn text-[12px]">The reviewed commit changed. Load a fresh agent review before posting.</span>}
    {event === "APPROVE" && <label className="flex items-center gap-2 text-[12px]"><input type="checkbox" checked={d.merge} disabled={busy} onChange={(e) => draft.set((v) => ({ ...v, merge: e.target.checked, runCi: false }))} />then squash and merge</label>}
    {d.runCi && <label className="flex items-center gap-2 text-[12px]"><input type="checkbox" checked={d.runCi} onChange={(e) => draft.set((v) => ({ ...v, runCi: e.target.checked, merge: false }))} />then approve waiting CI runs</label>}
    {!!error && <ErrorState error={error} />}
    {attempt && !busy && <button className="btn btn-sm" onClick={() => { void (async () => { if (attempt.source && suggestions.some((s) => s.id === attempt.source)) { await api.delete(`/api/suggestions/${attempt.source}`); void qc.invalidateQueries({ queryKey: ["suggestions"] }); } await draft.clear(); })().catch(setError); }}>I checked: review was posted</button>}
    {attempt && !busy && <div className="text-[12px]">Check GitHub before another attempt. <ConfirmButton consequence="Only continue if the review was not posted." confirmLabel="Start a new attempt" onConfirm={async () => { await abandonOperation(`${itemPath(repo, number)}/review`, attempt); if (attempt.source) { await api.delete(`/api/suggestions/${attempt.source}`); void qc.invalidateQueries({ queryKey: ["suggestions"] }); } draft.set((v) => ({ ...v, attempt: null, source: null })); setError(null); }}>I checked GitHub</ConfirmButton></div>}
    <div className="flex items-center gap-1">
      {event === "APPROVE" && d.merge ? <ConfirmButton className="btn-primary" disabled={disabled} consequence="Post this review, then squash and merge this commit?" confirmLabel="Approve and merge" onConfirm={post}>Approve and merge</ConfirmButton> : <button className="btn btn-sm btn-primary" disabled={disabled} onClick={() => void post()}>{busy ? "Posting…" : labels[event]}</button>}
      <Popover label="Review event" align="right" trigger={({ toggle }) => <button className="btn btn-sm" aria-label="Choose review event" disabled={busy} onClick={toggle}>▾</button>}>
        {(close) => <>{(Object.keys(labels) as ReviewEvent[]).map((e) => <MenuItem key={e} label={labels[e]} onClick={() => { preference.set(e); draft.set((v) => ({ ...v, event: e })); close(); }} />)}</>}
      </Popover>
      <ConfirmButton disabled={busy || !!attempt} consequence="Deletes the pending review summary and line comments. Agent suggestions stay available." confirmLabel="Discard review" onConfirm={() => draft.clear()}>Discard</ConfirmButton>
    </div>
  </section>;
}

function AgentDraft({ suggestion: s, head, busy, selected, onUse, onDrop, onError }: { suggestion: Suggestion; head: string; busy: boolean; selected: boolean; onUse: (v: Partial<PendingReview>) => void; onDrop: () => Promise<void>; onError: (e: unknown) => void }) {
  const edit = useReviewState<{ body?: string; event?: ReviewEvent; merge?: string | null; run_ci?: boolean }>(`suggestion:${s.id}`, {});
  const body = edit.value.body ?? s.body;
  const event = edit.value.event ?? s.event as ReviewEvent;
  return <div className="grid gap-2 rounded-card border border-line p-3 text-[12px]">
    <b>Agent draft · {s.author}</b><Markdown text={body} />
    {s.head !== head && <span className="text-attn">PR changed since this review.</span>}
    {s.status !== "draft" ? <span className="text-attn">Publication attempted. Check GitHub.</span> : <div className="flex gap-2">
      <button className="btn btn-sm" disabled={busy || s.head !== head || selected || !edit.loaded} onClick={() => onUse({ summary: body, event, merge: !!(edit.value.merge !== undefined ? edit.value.merge : s.merge), runCi: edit.value.run_ci ?? s.run_ci })}>{selected ? "In your review" : "Use"}</button>
      <button className="btn btn-sm" disabled={busy} onClick={() => void onDrop().catch(onError)}>Drop</button>
    </div>}
  </div>;
}

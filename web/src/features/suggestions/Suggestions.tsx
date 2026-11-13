import { Link } from "@tanstack/react-router";
import { LineComments } from "./LineComments";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useRef, useState } from "react";
import { api } from "@/lib/api";
import { useReviewState } from "@/lib/reviewState";
import { Markdown } from "@/components/Markdown";
import { openGitHub } from "@/features/github/lib";
import { Badge, ErrorState, Choice, Choices } from "@/components/ui";
import { ActionButton } from "@/components/ActionButton";
import { runAction } from "@/lib/actions";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import type { Suggestion } from "@/gen/Suggestion";
import type { SuggestionTarget } from "@/gen/SuggestionTarget";
export function useSuggestions(target: SuggestionTarget) {
  const q = useQuery({ queryKey: ["suggestions"], queryFn: () => api.get<Suggestion[]>("/api/suggestions") });
  return { ...q, data: (q.data ?? []).filter((s) => s.target.kind === target.kind && (s.target.kind === "predraft" && target.kind === "predraft" ? s.target.id === target.id : s.target.kind === "git_hub" && target.kind === "git_hub" && s.target.repo.toLowerCase() === target.repo.toLowerCase() && s.target.number === target.number)) };
}
/** A suggested comment or review (with its line comments): Post sends it all to GitHub in one click. A review can
 * also go to the pull request's review panel (Edit in pull request), to change its line comments there. */
export const SuggestionCard = memo(function SuggestionCard({ suggestion: s, head, onPosted }: { suggestion: Suggestion; head?: string; onPosted?: () => void }) {
  const qc = useQueryClient();
  // Unsaved edits live in the daemon so they follow the owner across devices.
  const edit = useReviewState<{ body?: string; event?: string; merge?: string | null; run_ci?: boolean; comments?: Suggestion["comments"] }>(`suggestion:${s.id}`, {});
  const [editingLocal, setEditing] = useState(false);
  const editing = editingLocal || Object.values(edit.value).some((v) => v !== undefined);
  const body = editing ? edit.value.body ?? s.body : s.body;
  const event = editing ? edit.value.event ?? s.event : s.event;
  const comments = editing ? edit.value.comments ?? s.comments : s.comments;
  // Follow-ups of a review, run by the same Post click: merge (approving reviews only) or run the waiting workflows.
  const merge = event === "APPROVE" ? (editing && edit.value.merge !== undefined ? edit.value.merge : s.merge) : null;
  const runCi = editing && edit.value.run_ci !== undefined ? edit.value.run_ci : s.run_ci;
  const followup = merge ? `then merge (${merge})` : runCi ? "then run CI" : "";
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const local = s.target.kind === "predraft";
  const reply = s.target.kind === "git_hub" ? s.target.reply_to ?? null : null;
  const stale = !local && s.kind === "review" && head !== undefined && s.head !== head;
  function clear() { void edit.clear(); }
  const save = () => api.patch(`/api/suggestions/${s.id}`, { body, event, merge, run_ci: runCi, comments });
  const posted = () => { toast(local ? "Suggestion saved to the Local PR notes" : s.kind === "review" ? (merge ? "Review posted and pull request merged" : runCi ? "Review posted and workflows approved" : "Suggested review posted") : "Suggested comment posted"); onPosted?.(); };
  const settled = () => { clear(); setEditing(false); void qc.invalidateQueries({ queryKey: ["suggestions"] }); };
  // A failed merge or workflow approval after a posted review removes the card from Today: the toast keeps it seen.
  const failed = (e: unknown) => { setError(e); toastError(e); onPosted?.(); void qc.invalidateQueries({ queryKey: ["suggestions"] }); };
  async function run(action: "save" | "post") {
    setBusy(true); setError(null);
    try {
      if (editing) await save();
      if (action === "post") { await runAction("suggestion.post", { id: s.id }); posted(); }
      clear(); setEditing(false);
    } catch (e) { setError(e); }
    finally { setBusy(false); void qc.invalidateQueries({ queryKey: ["suggestions"] }); }
  }
  return <article data-tour="suggestion.card" className="rounded-card border border-teal/40 bg-teal-soft p-4">
    <div className="mb-3 flex flex-wrap items-center gap-2 text-[12px]">
      <b className="text-teal-ink">Suggestion by {s.author}</b><span>{s.kind === "review" ? `Review · ${event === "APPROVE" ? "approve" : event === "REQUEST_CHANGES" ? "request changes" : "comment"}${followup && `, ${followup}`}` : local ? "Note" : reply ? <button className="text-teal-ink hover:underline" title={reply.url} onClick={() => openGitHub(reply.url)}>Reply to {reply.author ? `${reply.author}'s` : "a"} comment</button> : "Comment"}</span>
      {s.head && <span className="font-mono" title={s.head}>{s.head.slice(0, 7)}</span>}
      {s.status === "posted" && <Badge tone="ok" icon="check">{local ? "Saved to Local PR notes" : "Posted"}</Badge>}
      {s.status === "inserted" && <Badge tone="teal" icon="comment">Inserted on GitHub</Badge>}
      {s.status === "posted_by_hand" && <Badge tone="ok" icon="check">Posted by you on GitHub</Badge>}
    </div>
    {editing && s.status === "draft" ? <div className="grid gap-2">
      <textarea aria-label="Suggested text" className="field-input min-h-40" value={body} disabled={busy} onChange={(e) => { const v = e.target.value; edit.set((o) => ({ ...o, body: v })); }} />
      {s.kind === "review" && <Choices label="Suggested review type">{([["COMMENT", "Comment"], ["APPROVE", "Approve"], ["REQUEST_CHANGES", "Request changes"]] as const).map(([id, label]) => <Choice key={id} active={event === id} disabled={busy} onClick={() => edit.set((o) => ({ ...o, event: id }))}>{label}</Choice>)}</Choices>}
      {s.kind === "review" && !local && <Choices label="After posting">{([["none", "Nothing else"], ["merge", "Merge"], ["run_ci", "Run CI"]] as const).filter(([id]) => id !== "merge" || event === "APPROVE").map(([id, label]) => <Choice key={id} active={(id === "merge" ? !!merge : id === "run_ci" ? runCi && !merge : !merge && !runCi)} disabled={busy} onClick={() => edit.set((o) => ({ ...o, merge: id === "merge" ? merge ?? s.merge ?? "squash" : null, run_ci: id === "run_ci" }))}>{label}</Choice>)}</Choices>}
    </div> : <Markdown text={s.body} />}
    {!!comments.length && <div className="mt-3"><LineComments comments={comments} disabled={busy} onRemove={editing && s.status === "draft" ? (i) => edit.set((o) => ({ ...o, comments: (o.comments ?? s.comments).filter((_, j) => j !== i) })) : undefined} /></div>}
    {stale && s.status === "draft" && <p className="mt-3 flex items-center gap-1.5 text-[12px] text-attn"><Icon name="alert" size={12} />The PR head changed. Ask the agent for a fresh review before posting.</p>}
    {s.followup_error && <p className="mt-3 flex items-center gap-1.5 text-[12px] text-attn"><Icon name="alert" size={12} />{s.followup_error}</p>}
    {["posting", "uncertain"].includes(s.status) && <p className="mt-3 flex items-center gap-1.5 text-[12px] text-attn"><Icon name="alert" size={12} />Publication was attempted. Check GitHub before discarding or creating another suggestion. Automatic retries are disabled.</p>}
    {!!error && <ErrorState className="mt-2" error={error} />}
    <div className="mt-3 flex flex-wrap items-center gap-2">
      {s.status === "draft" && <>
        <button className="btn btn-sm" disabled={busy || (editing && !body.trim())} onClick={() => editing ? void run("save") : setEditing(true)}>{editing ? "Save edit" : "Edit"}</button>
        {editing && <button className="btn btn-sm" disabled={busy} onClick={() => { clear(); setEditing(false); }}>Cancel edit</button>}
        {local
          ? <button className="btn btn-primary btn-sm" disabled={busy || !body.trim()} onClick={() => void run("post")}>{busy ? "Working…" : "Add to local notes"}</button>
          : <ActionButton name="suggestion.post" params={{ id: s.id }} consequence={merge ? `Posts this review to GitHub as you, then merges the pull request (${merge}) at the reviewed commit.` : runCi ? "Posts this review to GitHub as you, then approves the waiting GitHub Actions runs of the reviewed commit: the pull request's code runs in the repository's CI." : undefined} primary disabled={busy || stale || !body.trim()} before={async () => { setError(null); if (editing) await save(); }} onDone={() => { posted(); settled(); }} onError={failed}>{s.kind === "review" ? (merge ? "Post review and merge" : runCi ? "Post review and run CI" : comments.length ? `Post review (${comments.length} line ${comments.length === 1 ? "comment" : "comments"})` : "Post review") : "Post comment"}</ActionButton>}
        {!local && s.kind === "review" && s.target.kind === "git_hub" && <Link className="btn btn-sm no-underline" to="/github/$owner/$repo/$number" params={{ owner: s.target.repo.split("/")[0], repo: s.target.repo.split("/")[1], number: String(s.target.number) }} search={{ tab: "files", draft: s.id }}>Edit in pull request</Link>}
        <span className="text-[12px] text-ink-3">{local ? "Stays in Launchpad." : "Posts to GitHub when you click."}</span>
      </>}
      {/* Insert in the Chrome extension claimed it: GitHub's own button posts it, never Launchpad's Post. */}
      {s.status === "inserted" && <span className="text-[12px] text-ink-3">In GitHub's comment box: post it there with GitHub's button.</span>}
      {s.status !== "posted" && s.status !== "posted_by_hand" && <ActionButton name="suggestion.discard" params={{ id: s.id }} disabled={busy || s.status === "posting"} consequence={["posting", "uncertain"].includes(s.status) ? "Check GitHub first: it may already be posted." : undefined} onDone={() => { toast("Suggestion discarded"); settled(); }} onError={failed} />}
      {s.url && <button className="btn btn-sm" onClick={() => openGitHub(s.url!)}><Icon name="ext" size={12} />Open posted text</button>}
    </div>
  </article>;
});
export function LocalSuggestions({ id }: { id: string }) {
  const q = useSuggestions({ kind: "predraft", id });
  const ref = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({ count: q.data.length, getScrollElement: () => ref.current, getItemKey: (i) => q.data[i].id, estimateSize: () => 230, overscan: 2, useFlushSync: false });
  if (q.error) return <ErrorState error={q.error} />;
  if (!q.data.length) return null;
  return <section className="mt-5"><h3 className="mb-2 font-semibold">Suggestions ({q.data.length})</h3><div ref={ref} className="max-h-[500px] overflow-auto" style={{ height: Math.min(500, v.getTotalSize()) }}><div className="relative" style={{ height: v.getTotalSize() }}>{v.getVirtualItems().map((r) => <div key={r.key} data-index={r.index} ref={v.measureElement} className="absolute top-0 left-0 w-full pb-3" style={{ transform: `translateY(${r.start}px)` }}><SuggestionCard suggestion={q.data[r.index]} /></div>)}</div></div></section>;
}

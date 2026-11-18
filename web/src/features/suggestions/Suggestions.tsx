import { LineComments } from "./LineComments";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useRef, useState } from "react";
import { api } from "@/lib/api";
import { useReviewState } from "@/lib/reviewState";
import { Markdown } from "@/components/Markdown";
import { openGitHub, suggestionItemUrl } from "@/lib/githubLinks";
import { Badge, ErrorState, Choice, Choices } from "@/components/ui";
import { ActionButton } from "@/components/ActionButton";
import { runAction } from "@/lib/actions";
import { Icon } from "@/components/Icon";
import { toast, toastError } from "@/components/Toast";
import type { Suggestion } from "@/gen/Suggestion";
import type { SuggestionTarget } from "@/gen/SuggestionTarget";
import type { ItemAction } from "@/gen/ItemAction";
import { describeAction, joined, parseReviewers, postButton, ranked, reviewersText } from "@/lib/suggestionActions";
export function useSuggestions(target: SuggestionTarget) {
  const q = useQuery({ queryKey: ["suggestions"], queryFn: () => api.get<Suggestion[]>("/api/suggestions") });
  return { ...q, data: (q.data ?? []).filter((s) => s.target.kind === target.kind && (s.target.kind === "predraft" && target.kind === "predraft" ? s.target.id === target.id : s.target.kind === "git_hub" && target.kind === "git_hub" && s.target.repo.toLowerCase() === target.repo.toLowerCase() && s.target.number === target.number)) };
}
/** The actions the owner may tick for a suggestion: the agent's own, plus a close for a comment that has none (the
 * older Comment and close switch). */
function actionOptions(s: Suggestion): ItemAction[] {
  if (s.target.kind !== "git_hub") return [];
  const extra: ItemAction[] = s.kind === "comment" && !s.actions.some((a) => a.kind === "close") ? [{ kind: "close", reason: null, duplicate_of: null }] : [];
  return ranked([...s.actions, ...extra]);
}

/** Ticks of the actions a suggestion runs after posting (each one off, or on as the agent wrote it), and the
 * reviewers of a review request, which the owner edits. */
function ActionTicks({ options, actions, alone, disabled, onChange }: { options: ItemAction[]; actions: ItemAction[]; alone: boolean; disabled: boolean; onChange: (next: ItemAction[]) => void }) {
  if (!options.length) return null;
  const on = (o: ItemAction) => actions.find((a) => a.kind === o.kind);
  const set = (o: ItemAction, value: ItemAction | null) => onChange(ranked([...actions.filter((a) => a.kind !== o.kind), ...(value ? [value] : [])]));
  const label = alone ? "Actions" : "After posting, also";
  return <div className="mt-3 grid gap-1.5 text-[12px]" aria-label={label}>
    <span className="text-ink-3">{label}:</span>
    {options.map((o) => {
      const current = on(o);
      return <div key={o.kind} className="flex flex-wrap items-center gap-2">
        <label className="flex items-center gap-2"><input type="checkbox" checked={!!current} disabled={disabled} onChange={(e) => set(o, e.target.checked ? o : null)} />{o.kind === "request_reviewers" && current ? "request reviews from" : describeAction(current ?? o)}</label>
        {o.kind === "request_reviewers" && current?.kind === "request_reviewers" && <input aria-label="Reviewers" className="field-input w-64" disabled={disabled} defaultValue={reviewersText(current)} onBlur={(e) => { const r = parseReviewers(e.target.value); set(o, r.reviewers.length || r.team_reviewers.length ? { kind: "request_reviewers", ...r } : null); }} />}
      </div>;
    })}
  </div>;
}

/** A suggested comment or review (with its line comments), or only actions: Post sends it all to GitHub in one click
 * (Home checks that a review's pull request is still at the reviewed commit), then runs the ticked actions once. Open
 * on GitHub shows the item there. */
export const SuggestionCard = memo(function SuggestionCard({ suggestion: s }: { suggestion: Suggestion }) {
  const qc = useQueryClient();
  // Unsaved edits live in the daemon so they follow the owner across devices. Ticking actions is not editing the text.
  const edit = useReviewState<{ body?: string; event?: string; merge?: string | null; run_ci?: boolean; comments?: Suggestion["comments"]; actions?: ItemAction[] }>(`suggestion:${s.id}`, {});
  const [editingLocal, setEditing] = useState(false);
  const { actions: ticked, ...textEdits } = edit.value;
  const editing = editingLocal || Object.values(textEdits).some((v) => v !== undefined);
  const body = editing ? edit.value.body ?? s.body : s.body;
  const event = editing ? edit.value.event ?? s.event : s.event;
  const comments = editing ? edit.value.comments ?? s.comments : s.comments;
  // Follow-ups of a review, run by the same Post click: merge (approving reviews only) or run the waiting workflows.
  const merge = event === "APPROVE" ? (editing && edit.value.merge !== undefined ? edit.value.merge : s.merge) : null;
  const runCi = editing && edit.value.run_ci !== undefined ? edit.value.run_ci : s.run_ci;
  const local = s.target.kind === "predraft";
  const onlyActions = s.kind === "action";
  // The actions Post runs after the text: the agent's (`--close`, `--request-reviewers`…), as the owner ticked them.
  const actions = local ? [] : ticked ?? s.actions;
  const options = actionOptions(s);
  const followup = [merge ? `merge (${merge})` : runCi ? "run CI" : "", ...actions.map(describeAction)].filter(Boolean);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const reply = s.target.kind === "git_hub" ? s.target.reply_to ?? null : null;
  function clear() { void edit.clear(); }
  const save = () => api.patch(`/api/suggestions/${s.id}`, { body, event, merge, run_ci: runCi, comments, ...(local ? {} : { actions }) });
  const posted = () => { toast(local ? "Suggestion saved to the Local PR notes" : onlyActions ? "Done on GitHub" : `${s.kind === "review" ? "Review" : "Comment"} posted${followup.length ? ` and ${joined(followup.map((f) => f.replace(/^request reviews from .*/, "reviews requested")))}` : ""}`); };
  const settled = () => { clear(); setEditing(false); void qc.invalidateQueries({ queryKey: ["suggestions"] }); };
  // A failed merge, workflow approval or action after a posted review removes the card from Today: the toast keeps it seen.
  const failed = (e: unknown) => { setError(e); toastError(e); void qc.invalidateQueries({ queryKey: ["suggestions"] }); };
  async function run(action: "save" | "post") {
    setBusy(true); setError(null);
    try {
      if (editing) await save();
      if (action === "post") { await runAction("suggestion.post", { id: s.id }); posted(); }
      clear(); setEditing(false);
    } catch (e) { setError(e); }
    finally { setBusy(false); void qc.invalidateQueries({ queryKey: ["suggestions"] }); }
  }
  const what = s.kind === "review" ? `Review · ${event === "APPROVE" ? "approve" : event === "REQUEST_CHANGES" ? "request changes" : "comment"}` : onlyActions ? "Actions only, no comment" : local ? "Note" : null;
  const base = s.kind === "review" ? (merge ? `Approve and merge (${merge})` : event === "APPROVE" ? "Approve" : event === "REQUEST_CHANGES" ? "Request changes" : comments.length ? `Post review (${comments.length} line ${comments.length === 1 ? "comment" : "comments"})` : "Post review") + (runCi ? " and run CI" : "") : onlyActions ? "" : reply ? "Post reply" : "Post comment";
  const textPart = s.kind === "review" ? "Posts this review to GitHub as you" : onlyActions ? "Launchpad does this on GitHub as you" : "Posts this comment to GitHub as you";
  const then = [merge ? `merges the pull request (${merge}) at the reviewed commit` : runCi ? "approves the waiting GitHub Actions runs of the reviewed commit (the pull request's code runs in the repository's CI)" : "", ...actions.map(describeAction)].filter(Boolean);
  const consequence = onlyActions ? `${textPart}: ${joined(actions.map(describeAction))}.` : then.length ? `${textPart}, then ${joined(then)}.` : undefined;
  return <article data-tour="suggestion.card" className="rounded-card border border-teal/40 bg-teal-soft p-4">
    <div className="mb-3 flex flex-wrap items-center gap-2 text-[12px]">
      <b className="text-teal-ink">Suggestion by {s.author}</b><span>{what ?? (reply ? <button className="text-teal-ink hover:underline" title={reply.url} onClick={() => openGitHub(reply.url)}>Reply to {reply.author ? `${reply.author}'s` : "a"} comment</button> : "Comment")}{!onlyActions && followup.length > 0 && `, then ${joined(followup)}`}</span>
      {s.head && <span className="font-mono" title={s.head}>{s.head.slice(0, 7)}</span>}
      {s.status === "posted" && <Badge tone="ok" icon="check">{local ? "Saved to Local PR notes" : onlyActions ? "Done" : "Posted"}</Badge>}
      {s.status === "inserted" && <Badge tone="teal" icon="comment">Inserted on GitHub</Badge>}
      {s.status === "posted_by_hand" && <Badge tone="ok" icon="check">Posted by you on GitHub</Badge>}
    </div>
    {editing && s.status === "draft" ? <div className="grid gap-2">
      <textarea aria-label="Suggested text" className="field-input min-h-40" value={body} disabled={busy} onChange={(e) => { const v = e.target.value; edit.set((o) => ({ ...o, body: v })); }} />
      {s.kind === "review" && <Choices label="Suggested review type">{([["COMMENT", "Comment"], ["APPROVE", "Approve"], ["REQUEST_CHANGES", "Request changes"]] as const).map(([id, label]) => <Choice key={id} active={event === id} disabled={busy} onClick={() => edit.set((o) => ({ ...o, event: id }))}>{label}</Choice>)}</Choices>}
      {s.kind === "review" && !local && <Choices label="After posting">{([["none", "Nothing else"], ["merge", "Merge"], ["run_ci", "Run CI"]] as const).filter(([id]) => id !== "merge" || event === "APPROVE").map(([id, label]) => <Choice key={id} active={(id === "merge" ? !!merge : id === "run_ci" ? runCi && !merge : !merge && !runCi)} disabled={busy} onClick={() => edit.set((o) => ({ ...o, merge: id === "merge" ? merge ?? s.merge ?? "squash" : null, run_ci: id === "run_ci" }))}>{label}</Choice>)}</Choices>}
    </div> : onlyActions ? <p className="text-[13px]">No comment: only the actions below.</p> : <Markdown text={s.body} />}
    {!!comments.length && <div className="mt-3"><LineComments comments={comments} disabled={busy} onRemove={editing && s.status === "draft" ? (i) => edit.set((o) => ({ ...o, comments: (o.comments ?? s.comments).filter((_, j) => j !== i) })) : undefined} /></div>}
    {s.status === "draft" && !local && <ActionTicks options={options} actions={actions} alone={onlyActions} disabled={busy} onChange={(next) => edit.set((o) => ({ ...o, actions: next }))} />}
    {s.followup_error && <p className="mt-3 flex items-center gap-1.5 text-[12px] text-attn"><Icon name="alert" size={12} />{s.followup_error}</p>}
    {["posting", "uncertain"].includes(s.status) && <p className="mt-3 flex items-center gap-1.5 text-[12px] text-attn"><Icon name="alert" size={12} />Publication was attempted. Check GitHub before discarding or creating another suggestion. Automatic retries are disabled.</p>}
    {!!error && <ErrorState className="mt-2" error={error} />}
    <div className="mt-3 flex flex-wrap items-center gap-2">
      {s.status === "draft" && <>
        {!onlyActions && <button className="btn btn-sm" disabled={busy || (editing && !body.trim())} onClick={() => editing ? void run("save") : setEditing(true)}>{editing ? "Save edit" : "Edit"}</button>}
        {editing && <button className="btn btn-sm" disabled={busy} onClick={() => { clear(); setEditing(false); }}>Cancel edit</button>}
        {local
          ? <button className="btn btn-primary btn-sm" disabled={busy || !body.trim()} onClick={() => void run("post")}>{busy ? "Working…" : "Add to local notes"}</button>
          : <ActionButton name="suggestion.post" params={{ id: s.id }} body={() => ({ actions })} consequence={consequence} primary disabled={busy || (onlyActions ? !actions.length : !body.trim())} before={async () => { setError(null); if (editing) await save(); }} onDone={() => { posted(); settled(); }} onError={failed}>{postButton(base, actions)}</ActionButton>}
        {!local && suggestionItemUrl(s) && <button className="btn btn-sm" onClick={() => openGitHub(suggestionItemUrl(s)!)}><Icon name="ext" size={12} />Open on GitHub</button>}
        <span className="text-[12px] text-ink-3">{local ? "Stays in Launchpad." : "Posts to GitHub when you click."}</span>
      </>}
      {/* Insert in the Chrome extension claimed it: GitHub's own button posts it, never Launchpad's Post. */}
      {s.status === "inserted" && <span className="text-[12px] text-ink-3">In GitHub's comment box: post it there with GitHub's button.</span>}
      {s.status !== "posted" && s.status !== "posted_by_hand" && <ActionButton name="suggestion.discard" params={{ id: s.id }} disabled={busy || s.status === "posting"} consequence={["posting", "uncertain"].includes(s.status) ? "Check GitHub first: it may already be posted." : undefined} onDone={() => { toast("Suggestion discarded"); settled(); }} onError={failed} />}
      {s.url && <button className="btn btn-sm" onClick={() => openGitHub(s.url!)}><Icon name="ext" size={12} />{onlyActions ? "Open on GitHub" : "Open posted text"}</button>}
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

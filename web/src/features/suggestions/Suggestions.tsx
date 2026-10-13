import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useRef, useState } from "react";
import { api } from "@/lib/api";
import { useReviewState } from "@/lib/reviewState";
import { Markdown } from "@/components/Markdown";
import { openGitHub } from "@/features/github/lib";
import type { Suggestion } from "@/gen/Suggestion";
import type { SuggestionTarget } from "@/gen/SuggestionTarget";
export function useSuggestions(target: SuggestionTarget) {
  const q = useQuery({ queryKey: ["suggestions"], queryFn: () => api.get<Suggestion[]>("/api/suggestions") });
  return { ...q, data: (q.data ?? []).filter((s) => s.target.kind === target.kind && (s.target.kind === "predraft" && target.kind === "predraft" ? s.target.id === target.id : s.target.kind === "git_hub" && target.kind === "git_hub" && s.target.repo.toLowerCase() === target.repo.toLowerCase() && s.target.number === target.number)) };
}
export const SuggestionCard = memo(function SuggestionCard({ suggestion: s, head, onPosted }: { suggestion: Suggestion; head?: string; onPosted?: () => void }) {
  const qc = useQueryClient();
  // Unsaved edits live in the daemon so they follow the owner across devices.
  const edit = useReviewState<{ body?: string; event?: string }>(`suggestion:${s.id}`, {});
  const [editingLocal, setEditing] = useState(false);
  const editing = editingLocal || edit.value.body !== undefined || edit.value.event !== undefined;
  const body = editing ? edit.value.body ?? s.body : s.body;
  const event = editing ? edit.value.event ?? s.event : s.event;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const local = s.target.kind === "predraft";
  const stale = !local && s.kind === "review" && head !== undefined && s.head !== head;
  function clear() { void edit.clear(); }
  async function run(action: "save" | "post" | "discard") {
    setBusy(true); setError("");
    try {
      if (action === "discard") await api.delete(`/api/suggestions/${s.id}`);
      else {
        if (editing) await api.patch(`/api/suggestions/${s.id}`, { body, event });
        if (action === "post") { await api.post(`/api/suggestions/${s.id}/post`); onPosted?.(); }
      }
      clear(); setEditing(false);
    } catch (e) { setError(String(e)); }
    finally { setBusy(false); void qc.invalidateQueries({ queryKey: ["suggestions"] }); }
  }
  return <article className="rounded-lg border border-teal-ink/30 bg-teal-soft p-4">
    <div className="mb-3 flex flex-wrap items-center gap-2 text-xs">
      <b className="text-teal-ink">Suggested by {s.author}</b><span>{s.kind}{s.kind === "review" && ` · ${event.toLowerCase().replaceAll("_", " ")}`}</span>
      {s.head && <span className="font-mono" title={s.head}>{s.head.slice(0, 7)}</span>}
      {s.status === "posted" && <b>{local ? "Saved to local notes" : "Posted"}</b>}
    </div>
    {editing && s.status === "draft" ? <div className="grid gap-2">
      <textarea aria-label="Suggested text" className="field-input min-h-40" value={body} disabled={busy} onChange={(e) => { const v = e.target.value; edit.set((o) => ({ ...o, body: v })); }} />
      {s.kind === "review" && <select aria-label="Suggested review type" className="field-input" value={event} disabled={busy} onChange={(e) => { const v = e.target.value; edit.set((o) => ({ ...o, event: v })); }}><option value="COMMENT">Comment review</option><option value="APPROVE">Approve</option><option value="REQUEST_CHANGES">Request changes</option></select>}
    </div> : <Markdown text={s.body} />}
    {stale && s.status === "draft" && <p className="mt-3 text-xs text-amber-800">PR head changed. Ask the agent for a fresh review before posting.</p>}
    {["posting", "uncertain"].includes(s.status) && <p className="mt-3 text-xs text-amber-800">Publication was attempted. Check GitHub before discarding or creating another suggestion. Automatic retries are disabled.</p>}
    {error && <p role="alert" className="mt-2 text-gh-closed">{error}</p>}
    <div className="mt-3 flex flex-wrap items-center gap-2">
      {s.status === "draft" && <>
        <button className="btn btn-sm" disabled={busy || (editing && !body.trim())} onClick={() => editing ? void run("save") : setEditing(true)}>{editing ? "Save edit" : "Edit"}</button>
        {editing && <button className="btn btn-sm" disabled={busy} onClick={() => { clear(); setEditing(false); }}>Cancel edit</button>}
        <button className="btn btn-primary btn-sm" disabled={busy || stale || !body.trim()} onClick={() => void run("post")}>{busy ? "Working…" : local ? "Add to local notes" : s.kind === "review" ? "Post review" : "Post comment"}</button>
        <span className="text-xs text-ink-3">{local ? "Stays on this machine." : "Posts to GitHub when you click."}</span>
      </>}
      {s.status !== "posted" && <button className="btn btn-sm" disabled={busy} onClick={() => void run("discard")}>Discard</button>}
      {s.url && <button className="btn btn-sm" onClick={() => openGitHub(s.url!)}>Open posted text</button>}
    </div>
  </article>;
});
export function LocalSuggestions({ id }: { id: string }) {
  const q = useSuggestions({ kind: "predraft", id });
  const ref = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({ count: q.data.length, getScrollElement: () => ref.current, getItemKey: (i) => q.data[i].id, estimateSize: () => 230, overscan: 2, useFlushSync: false });
  if (q.error) return <p role="alert" className="text-gh-closed">{String(q.error)}</p>;
  if (!q.data.length) return null;
  return <section className="mt-5"><h3 className="mb-2 font-semibold">Agent suggestions ({q.data.length})</h3><div ref={ref} className="max-h-[500px] overflow-auto" style={{ height: Math.min(500, v.getTotalSize()) }}><div className="relative" style={{ height: v.getTotalSize() }}>{v.getVirtualItems().map((r) => <div key={r.key} data-index={r.index} ref={v.measureElement} className="absolute top-0 left-0 w-full pb-3" style={{ transform: `translateY(${r.start}px)` }}><SuggestionCard suggestion={q.data[r.index]} /></div>)}</div></div></section>;
}

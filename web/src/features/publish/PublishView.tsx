// Everything agents prepared for GitHub that waits for an owner click. Nothing is posted from here.
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useMemo, useRef, useState } from "react";
import type { Predraft } from "@/gen/Predraft";
import type { Project } from "@/gen/Project";
import type { Session } from "@/gen/Session";
import type { Suggestion } from "@/gen/Suggestion";
import { HarnessMark, useHarnessName } from "@/components/HarnessMark";
import { toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { formatAge } from "@/lib/format";
import { useProjects, useSessions } from "@/lib/queries";
import { useScope } from "@/lib/workspace";

type Row = { kind: "suggestion"; s: Suggestion; at: number } | { kind: "predraft"; p: Predraft; at: number };

/** Unposted suggestions and unpublished pre-drafts of the workspace, newest first. The sidebar count uses the same list. */
export function usePublishItems(): Row[] {
  const scope = useScope();
  const suggestions = useQuery({ queryKey: ["suggestions"], queryFn: () => api.get<Suggestion[]>("/api/suggestions") }).data;
  const predrafts = useQuery({ queryKey: ["predrafts"], queryFn: () => api.get<Predraft[]>("/api/predrafts") }).data;
  return useMemo(() => {
    const open = (predrafts ?? []).filter((p) => !p.archived && !p.published_url);
    const owner = new Map((predrafts ?? []).map((p) => [p.id, p.project_id]));
    const rows: Row[] = [
      ...(suggestions ?? []).filter((s) => s.status !== "posted" && (s.target.kind === "git_hub" ? scope.hasRepo(s.target.repo) : scope.hasProject(owner.get(s.target.id) ?? null))).map((s) => ({ kind: "suggestion" as const, s, at: s.created_at })),
      ...open.filter((p) => scope.hasProject(p.project_id)).map((p) => ({ kind: "predraft" as const, p, at: p.created_at })),
    ];
    return rows.sort((a, b) => b.at - a.at);
  }, [suggestions, predrafts, scope]);
}

function Source({ session }: { session: Session | undefined }) {
  const name = useHarnessName();
  if (!session) return null;
  return <span className="flex min-w-0 items-center gap-1">From <Link className="truncate" to="/agents" search={{ session: session.id }} title={session.title}>"{session.title}"</Link><HarnessMark harness={session.harness} size={12} /><span className="flex-none">{name(session.harness)}</span></span>;
}

const event = (e: string) => e === "APPROVE" ? "Approve" : e === "REQUEST_CHANGES" ? "Request changes" : "Comment";

const SuggestionRow = memo(function SuggestionRow({ s, session, predraft }: { s: Suggestion; session: Session | undefined; predraft: Predraft | undefined }) {
  const qc = useQueryClient();
  const [confirm, setConfirm] = useState(false);
  const [busy, setBusy] = useState(false);
  const target = s.target.kind === "git_hub" ? `${s.target.repo} #${s.target.number}` : `pre-draft "${predraft?.title ?? s.target.id}"`;
  const discard = async () => {
    setBusy(true);
    try { await api.delete(`/api/suggestions/${s.id}`); await qc.invalidateQueries({ queryKey: ["suggestions"] }); }
    catch (e) { toastError(e); }
    finally { setBusy(false); setConfirm(false); }
  };
  const t = s.target;
  return <div className="flex h-[64px] items-center gap-3 border-b border-line-2 px-4">
    <span className={`flex size-7 flex-none items-center justify-center rounded-md text-[11px] font-bold ${s.kind === "review" ? "bg-attn-soft text-attn" : "bg-select text-teal-ink"}`}>{s.kind === "review" ? "R" : "C"}</span>
    <div className="min-w-0 flex-1">
      <div className="truncate font-semibold">{s.kind === "review" ? `Review on ${target} · ${event(s.event)}` : `${t.kind === "predraft" ? "Note" : "Comment"} on ${target}`}{s.status !== "draft" && <span className="ml-2 text-xs font-normal text-amber-800">publication {s.status}: check GitHub</span>}</div>
      <div className="mt-0.5 flex min-w-0 items-center gap-1.5 text-[11.5px] text-ink-3">
        <Source session={session} />
        {!session && <span className="flex-none">From {s.author}</span>}
        {s.head && <span className="flex-none">· inspected <span className="font-mono" title={s.head}>{s.head.slice(0, 7)}</span></span>}
        <span className="flex-none">· {formatAge(s.created_at)} ago</span>
      </div>
    </div>
    {t.kind === "git_hub"
      ? <Link className="btn btn-sm btn-primary no-underline" to="/github/$owner/$repo/$number" params={{ owner: t.repo.split("/")[0], repo: t.repo.split("/")[1], number: String(t.number) }} search={{ draft: s.id }}>Review &amp; post</Link>
      : <Link className="btn btn-sm btn-primary no-underline" to="/predrafts/$id" params={{ id: t.id }}>Review &amp; post</Link>}
    {confirm
      ? <><button className="btn btn-sm text-gh-closed" disabled={busy} onClick={() => void discard()}>Confirm discard</button><button className="btn btn-sm" disabled={busy} onClick={() => setConfirm(false)}>Cancel</button></>
      : <button className="btn btn-sm" disabled={s.status === "posting"} onClick={() => setConfirm(true)}>Discard</button>}
  </div>;
});

const PredraftRow = memo(function PredraftRow({ p, session, project }: { p: Predraft; session: Session | undefined; project: Project | undefined }) {
  return <div className="flex h-[64px] items-center gap-3 border-b border-line-2 px-4">
    <span className="flex size-7 flex-none items-center justify-center rounded-md bg-sunk text-[11px] font-bold text-ink-2">PR</span>
    <div className="min-w-0 flex-1">
      <div className="truncate font-semibold">Pre-draft PR · {p.title}</div>
      <div className="mt-0.5 flex min-w-0 items-center gap-1.5 text-[11.5px] text-ink-3">
        <span className="flex-none">{project?.name ?? "missing project"} · <span className="font-mono">{p.branch}</span> ·</span>
        <Source session={session} />
        <span className="flex-none">· {formatAge(p.created_at)} ago</span>
      </div>
    </div>
    <Link className="btn btn-sm btn-primary no-underline" to="/predrafts/$id" params={{ id: p.id }}>Review &amp; publish</Link>
    {project && <Link className="btn btn-sm no-underline" to="/projects/$id/code" params={{ id: project.id }} search={{ root: p.root }}>Open worktree</Link>}
  </div>;
});

export function PublishView() {
  const rows = usePublishItems();
  const sessions = useSessions().data;
  const projects = useProjects().data;
  const predrafts = useQuery({ queryKey: ["predrafts"], queryFn: () => api.get<Predraft[]>("/api/predrafts") }).data;
  const bySession = useMemo(() => new Map((sessions ?? []).map((s) => [s.id, s])), [sessions]);
  const ref = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({ count: rows.length, getScrollElement: () => ref.current, estimateSize: () => 64, getItemKey: (i) => { const r = rows[i]; return r.kind === "suggestion" ? r.s.id : r.p.id; }, overscan: 4, useFlushSync: false });
  return <div className="flex h-full flex-col">
    <div className="flex h-14 flex-none items-center gap-3 border-b border-line px-4">
      <h1 className="text-lg font-semibold">To publish</h1>
      <span className="text-xs text-ink-3">{rows.length} {rows.length === 1 ? "item" : "items"} · nothing is posted without your click</span>
    </div>
    <div ref={ref} className="min-h-0 flex-1 overflow-y-auto">
      {rows.length === 0 && <p className="p-8 text-center text-ink-3">No agent drafts or pre-draft PRs wait for you.</p>}
      <div className="relative" style={{ height: v.getTotalSize() }}>
        {v.getVirtualItems().map((item) => {
          const r = rows[item.index];
          return <div key={item.key} className="absolute top-0 left-0 w-full" style={{ transform: `translateY(${item.start}px)` }}>
            {r.kind === "suggestion"
              ? <SuggestionRow s={r.s} session={r.s.session_id ? bySession.get(r.s.session_id) : undefined} predraft={r.s.target.kind === "predraft" ? predrafts?.find((p) => r.s.target.kind === "predraft" && p.id === r.s.target.id) : undefined} />
              : <PredraftRow p={r.p} session={r.p.session_id ? bySession.get(r.p.session_id) : undefined} project={projects?.find((x) => x.id === r.p.project_id)} />}
          </div>;
        })}
      </div>
    </div>
  </div>;
}

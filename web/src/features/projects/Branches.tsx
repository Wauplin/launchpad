// A folder's Branches tab: the agent branches of this folder with work to look at (commits ahead of their base,
// uncommitted files or a merge request), newest first. Each row opens the agent's Changes; Merge stays on the
// agent's worktree card. Today lists a finished agent's branch until you have seen it.

import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useMemo, useRef } from "react";
import type { Session } from "@/gen/Session";
import type { WorktreeInfo } from "@/gen/WorktreeInfo";
import { Icon } from "@/components/Icon";
import { Badge, Empty, Loading } from "@/components/ui";
import { StatusBadge } from "@/features/agents/SessionBits";
import { api } from "@/lib/api";
import { ago } from "@/lib/format";
import { usePhone } from "@/lib/phone";
import { useSessions } from "@/lib/queries";
import { offlineWorktreesNote, useOfflineRunners } from "@/lib/runners";

const BranchRow = memo(function BranchRow({ i, session }: { i: WorktreeInfo; session: Session | undefined }) {
  const phone = usePhone();
  const w = i.worktree;
  const state = w.merge ? "merge requested" : w.push_note ? "not on Home" : null;
  return <div className={`grid items-center gap-x-3 border-b border-line-2 px-4 ${phone ? "h-[112px] grid-cols-[32px_minmax(0,1fr)] content-center gap-y-2" : "h-14 grid-cols-[32px_minmax(0,1fr)_auto]"}`}>
    <span className="grid size-8 place-items-center rounded-lg bg-sunk text-ink-2"><Icon name="branch" size={15} /></span>
    <div className="min-w-0">
      <div className="flex min-w-0 items-center gap-2"><span className="truncate text-[13px] font-semibold">{session?.title ?? w.branch}</span>{session && <StatusBadge s={session} />}</div>
      <div className="mt-0.5 flex min-w-0 items-center gap-1.5 truncate text-[12px] text-ink-3">
        <span className="truncate font-mono">{w.branch}</span>
        {i.ahead > 0 && <span className="flex-none">· {i.ahead} commit{i.ahead === 1 ? "" : "s"} ahead</span>}
        {i.dirty_files > 0 && <span className="flex-none">· {i.dirty_files} uncommitted</span>}
        <span className="flex-none">· {ago(w.created_at)}</span>
        {state && <Badge tone="attn">{state}</Badge>}
      </div>
    </div>
    {session && <div className={`flex items-center justify-end gap-2 ${phone ? "col-span-2 justify-start" : ""}`}>
      <Link className="btn btn-sm btn-primary no-underline" to="/agents" search={{ session: session.id, view: "changes" }}>Review changes</Link>
      <Link className="btn btn-sm no-underline" to="/agents" search={{ session: session.id }}>Open agent</Link>
    </div>}
  </div>;
});

export function Branches({ projectId }: { projectId: string }) {
  const phone = usePhone();
  const worktrees = useQuery({ queryKey: ["worktrees", "all"], queryFn: () => api.get<WorktreeInfo[]>("/api/worktrees"), refetchInterval: 15_000 }).data;
  const sessions = useSessions().data;
  const offline = offlineWorktreesNote(useOfflineRunners());
  const bySession = useMemo(() => new Map((sessions ?? []).map((s) => [s.id, s])), [sessions]);
  const rows = useMemo(() => (worktrees ?? [])
    .filter((i) => i.worktree.project_id === projectId && !i.worktree.predraft_id && (i.worktree.state === "active" || i.worktree.state === "removed") && (i.ahead > 0 || i.dirty_files > 0 || !!i.worktree.merge))
    .sort((a, b) => b.worktree.created_at - a.worktree.created_at), [worktrees, projectId]);
  const ref = useRef<HTMLDivElement>(null);
  const height = phone ? 112 : 56;
  const v = useVirtualizer({ count: rows.length, getScrollElement: () => ref.current, estimateSize: () => height, getItemKey: (i) => rows[i].worktree.id, overscan: 4, useFlushSync: false });
  if (!worktrees) return <Loading what="agent branches" />;
  return <div className="grid gap-2" data-tour="folder.branches">
    {offline && <div className="text-[12px] text-ink-3">{offline}</div>}
    {rows.length === 0 ? <Empty icon="branch">No agent branch with work to review or merge in this folder.</Empty> : (
      <div ref={ref} className="overflow-y-auto rounded-lg border border-line" style={{ height: Math.min(rows.length * height, 560) + 2 }}>
        <div className="relative" style={{ height: v.getTotalSize() }}>
          {v.getVirtualItems().map((item) => {
            const i = rows[item.index];
            return <div key={item.key} className="absolute top-0 left-0 w-full" style={{ transform: `translateY(${item.start}px)` }}><BranchRow i={i} session={bySession.get(i.worktree.sessions[i.worktree.sessions.length - 1])} /></div>;
          })}
        </div>
      </div>
    )}
  </div>;
}

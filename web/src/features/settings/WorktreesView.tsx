// Settings → Worktrees: retention of task worktrees and every recorded worktree with its state.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useEffect, useRef, useState } from "react";
import type { Config } from "@/gen/Config";
import type { WorktreeInfo } from "@/gen/WorktreeInfo";
import { toast, toastError } from "@/components/Toast";
import { api } from "@/lib/api";
import { useConfig, type ConfigResponse } from "@/lib/queries";
import { useLayout } from "@/stores/layout";
import { useNavigate } from "@tanstack/react-router";
import { worktreeState } from "@/features/agents/Worktrees";
import { Card, SaveBar, SettingsPage } from "./SettingsView";

const ROW = 58;

function size(bytes: number | undefined) {
  if (bytes === undefined) return null;
  return bytes > 1e9 ? `${(bytes / 1e9).toFixed(1)} GB` : `${Math.max(1, Math.round(bytes / 1e6))} MB`;
}

const WorktreeRow = memo(function WorktreeRow({ info: i, bytes }: { info: WorktreeInfo; bytes?: number }) {
  const w = i.worktree;
  const [busy, setBusy] = useState(false);
  const [confirm, setConfirm] = useState(false);
  const show = useLayout((s) => s.show);
  const navigate = useNavigate();
  const act = async (action: string) => {
    setBusy(true);
    try { await api.post(`/api/worktrees/${w.id}/${action}`); } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  return (
    <div className="flex h-[58px] items-center gap-3 border-b border-line-2 px-3 text-xs">
      <div className="grid min-w-0 flex-1 gap-0.5">
        <span className="flex min-w-0 items-center gap-1.5"><b className="truncate font-mono text-[11.5px]">{w.branch}</b><span className="shrink-0 text-ink-3">{i.project_name}</span></span>
        <span className="truncate text-ink-3" title={w.cleanup_note ?? i.path}>{worktreeState(i)}{w.cleanup_note && w.state === "active" ? ` · ${w.cleanup_note}` : ""}{size(bytes) ? ` · ${size(bytes)}` : ""}</span>
      </div>
      {w.sessions.length > 0 && <button className="btn btn-sm" onClick={() => { show(w.sessions[w.sessions.length - 1]); void navigate({ to: "/agents" }); }}>Agent</button>}
      {w.state === "active" && <button className="btn btn-sm" disabled={busy} title="Remove the clean folder; the branch keeps the commits and Resume restores it" onClick={() => void act("remove")}>Remove folder</button>}
      {w.state === "removed" && <button className="btn btn-sm" disabled={busy} onClick={() => void act("restore")}>Restore</button>}
      {(w.state === "active" || w.state === "removed") && <button className="btn btn-sm text-gh-closed" disabled={busy} title="Delete the folder and branch, including uncommitted and unmerged work" onClick={() => { if (!confirm) return setConfirm(true); setConfirm(false); void act("discard"); }}>{confirm ? "Confirm discard" : "Discard"}</button>}
    </div>
  );
});

export function WorktreesView() {
  const q = useConfig();
  const qc = useQueryClient();
  const [draft, setDraft] = useState<Config | null>(null);
  useEffect(() => { if (q.data && !draft) setDraft(q.data.config); }, [q.data, draft]);
  const list = useQuery({ queryKey: ["worktrees", "all"], queryFn: () => api.get<WorktreeInfo[]>("/api/worktrees") });
  const [sizes, setSizes] = useState<Record<string, number> | null>(null);
  const [measuring, setMeasuring] = useState(false);
  const [filter, setFilter] = useState<"present" | "all">("present");
  const rows = (list.data ?? []).filter((i) => filter === "all" || i.worktree.state === "active" || i.worktree.state === "handed_off" || !!i.worktree.merge);
  const scrollRef = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({ count: rows.length, getScrollElement: () => scrollRef.current, estimateSize: () => ROW, overscan: 8, useFlushSync: false });
  if (!draft || !q.data) return <SettingsPage><div className="text-ink-3">Loading…</div></SettingsPage>;
  const saved = q.data.config;
  const merged = { ...saved, worktrees: draft.worktrees };
  const dirty = JSON.stringify(merged.worktrees) !== JSON.stringify(saved.worktrees);
  const total = sizes ? Object.values(sizes).reduce((a, b) => a + b, 0) : null;
  const save = async () => {
    try {
      const res = await api.put<ConfigResponse>("/api/config", merged);
      qc.setQueryData(["config"], res);
      setDraft(res.config);
      toast("Settings saved");
    } catch (e) { toastError(e); }
  };
  return (
    <SettingsPage>
      <Card title="Worktrees">
        <p className="m-0 text-xs text-ink-2">New agents in a Git project run in their own worktree: a new branch and folder under <span className="font-mono">{saved.projects_root}/.worktrees</span>, sharing the repository history. Archiving an agent removes its clean worktree; older clean worktrees of idle agents are removed too. The branch keeps the commits and sending a message restores the folder. Worktrees with uncommitted work are never removed automatically.</p>
        <label className="grid grid-cols-[170px_1fr] items-center gap-3.5 text-[12.5px]">
          <span className="grid gap-0.5"><b className="font-[650]">Keep on disk</b><span className="text-[11.5px] text-ink-3">Clean worktrees of idle agents. 0 keeps all.</span></span>
          <input type="number" min={0} className="field-input w-24" value={draft.worktrees.keep} onChange={(e) => setDraft({ ...draft, worktrees: { keep: Math.max(0, Math.floor(Number(e.target.value) || 0)) } })} />
        </label>
        <p className="m-0 text-[11.5px] text-ink-3">Copy ignored files such as <span className="font-mono">.env</span> into new worktrees by listing them in a <span className="font-mono">.worktreeinclude</span> file at the repository root (same format as Codex). A setup command per project, such as <span className="font-mono">pnpm install</span>, is set on the project page.</p>
      </Card>
      <Card title="Task worktrees" end={<>
        <button className={`btn btn-sm ${filter === "present" ? "bg-select!" : ""}`} aria-pressed={filter === "present"} onClick={() => setFilter("present")}>On disk</button>
        <button className={`btn btn-sm ${filter === "all" ? "bg-select!" : ""}`} aria-pressed={filter === "all"} onClick={() => setFilter("all")}>All records</button>
        <button className="btn btn-sm" disabled={measuring} onClick={async () => { setMeasuring(true); try { setSizes(await api.post<Record<string, number>>("/api/worktrees/sizes")); } catch (e) { toastError(e); } finally { setMeasuring(false); } }}>{measuring ? "Measuring…" : "Measure disk usage"}</button>
      </>}>
        <div className="text-xs text-ink-3">{rows.length} worktree{rows.length === 1 ? "" : "s"}{total !== null ? ` · ${size(total)} on disk` : ""}</div>
        {rows.length === 0 ? <div className="text-xs text-ink-3">No task worktree yet.</div> : (
          <div ref={scrollRef} className="max-h-[480px] overflow-y-auto rounded-lg border border-line">
            <div style={{ height: v.getTotalSize(), position: "relative" }}>
              {v.getVirtualItems().map((it) => (
                <div key={rows[it.index].worktree.id} style={{ position: "absolute", top: 0, left: 0, right: 0, transform: `translateY(${it.start}px)` }}>
                  <WorktreeRow info={rows[it.index]} bytes={sizes?.[rows[it.index].worktree.id]} />
                </div>
              ))}
            </div>
          </div>
        )}
      </Card>
      <SaveBar dirty={dirty} onSave={() => void save()} onRevert={() => setDraft(saved)} />
    </SettingsPage>
  );
}

// Settings → Maintenance: worktree retention, Local PR cleanup and every recorded task worktree with its state.

import { useQuery } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useEffect, useRef, useState } from "react";
import type { WorktreeInfo } from "@/gen/WorktreeInfo";
import { toastError } from "@/components/Toast";
import { ConfirmButton, Empty, FilterChip, FilterChips, Loading, SettingRow, useSaveState } from "@/components/ui";
import { api } from "@/lib/api";
import { useConfig } from "@/lib/queries";
import { useLayout } from "@/stores/layout";
import { useNavigate } from "@tanstack/react-router";
import { worktreeState } from "@/features/agents/Worktrees";
import { Card, ConfigError, LocalPrCleanupRow, SettingsPage, usePatchConfig } from "./SettingsView";

const ROW = 56;

function size(bytes: number | undefined) {
  if (bytes === undefined) return null;
  return bytes > 1e9 ? `${(bytes / 1e9).toFixed(1)} GB` : `${Math.max(1, Math.round(bytes / 1e6))} MB`;
}

const WorktreeRow = memo(function WorktreeRow({ info: i, bytes }: { info: WorktreeInfo; bytes?: number }) {
  const w = i.worktree;
  const [busy, setBusy] = useState(false);
  const show = useLayout((s) => s.show);
  const navigate = useNavigate();
  const act = async (action: string) => {
    setBusy(true);
    try { await api.post(`/api/worktrees/${w.id}/${action}`); } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  return (
    <div className="flex h-[56px] items-center gap-3 border-b border-line-2 px-3 text-xs">
      <div className="grid min-w-0 flex-1 gap-0.5">
        <span className="flex min-w-0 items-center gap-1.5"><b className="truncate font-mono text-[12px]">{w.branch}</b><span className="shrink-0 text-ink-3">{i.project_name}</span></span>
        <span className="truncate text-ink-3" title={w.cleanup_note ?? i.path}>{worktreeState(i)}{w.cleanup_note && w.state === "active" ? ` · ${w.cleanup_note}` : ""}{size(bytes) ? ` · ${size(bytes)}` : ""}</span>
      </div>
      {w.sessions.length > 0 && <button className="btn btn-sm" onClick={() => { show(w.sessions[w.sessions.length - 1]); void navigate({ to: "/agents" }); }}>Agent</button>}
      {w.state === "active" && <button className="btn btn-sm" disabled={busy} title="Remove the clean folder; the branch keeps the commits and Resume restores it" onClick={() => void act("remove")}>Remove folder</button>}
      {w.state === "removed" && <button className="btn btn-sm" disabled={busy} onClick={() => void act("restore")}>Restore</button>}
      {(w.state === "active" || w.state === "removed") && <ConfirmButton disabled={busy} consequence="Deletes the folder and branch, including unmerged work." confirmLabel="Discard" onConfirm={() => act("discard")}>Discard</ConfirmButton>}
    </div>
  );
});

export function WorktreesView() {
  const q = useConfig();
  const patch = usePatchConfig();
  const keep = useSaveState();
  const list = useQuery({ queryKey: ["worktrees", "all"], queryFn: () => api.get<WorktreeInfo[]>("/api/worktrees") });
  const [sizes, setSizes] = useState<Record<string, number> | null>(null);
  const [measuring, setMeasuring] = useState(false);
  const [filter, setFilter] = useState<"present" | "all">("present");
  const [keepText, setKeepText] = useState<string | null>(null);
  const all = list.data ?? [];
  const present = (i: WorktreeInfo) => i.worktree.state === "active" || i.worktree.state === "handed_off" || !!i.worktree.merge;
  const rows = all.filter((i) => filter === "all" || present(i));
  const scrollRef = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({ count: rows.length, getScrollElement: () => scrollRef.current, estimateSize: () => ROW, overscan: 8, useFlushSync: false });
  useEffect(() => { if (q.data) setKeepText(String(q.data.config.worktrees.keep)); }, [q.data]);
  if (!q.data || keepText === null) return <SettingsPage title="Maintenance"><Loading what="settings" /></SettingsPage>;
  const saved = q.data.config;
  const total = sizes ? Object.values(sizes).reduce((a, b) => a + b, 0) : null;
  const commitKeep = () => {
    const n = Math.max(0, Math.floor(Number(keepText) || 0));
    setKeepText(String(n));
    if (n !== saved.worktrees.keep) void keep.run(() => patch({ worktrees_keep: n }));
  };
  return (
    <SettingsPage title="Maintenance" intro="Disk cleanup for task worktrees and Local PRs.">
      <ConfigError />
      <Card title="Cleanup">
        <SettingRow label="Worktrees kept on disk" hint="Clean worktrees of idle agents. 0 keeps all." scope="all" state={keep.state} error={keep.error}
          info={`New agents in a Git folder run in their own worktree under ${saved.projects_root}/.worktrees. Archiving an agent removes its clean worktree; older clean worktrees of idle agents are removed too. The branch keeps the commits and sending a message restores the folder. Worktrees with uncommitted work are never removed. List ignored files such as .env in a .worktreeinclude file to copy them into new worktrees; the setup command is set on the folder page.`}>
          <input type="number" min={0} aria-label="Worktrees kept on disk" className="field-input w-24" value={keepText} onChange={(e) => setKeepText(e.target.value)} onBlur={commitKeep} onKeyDown={(e) => { if (e.key === "Enter") commitKeep(); }} />
        </SettingRow>
        <LocalPrCleanupRow config={saved} />
      </Card>
      <Card title="Task worktrees" end={<>
        <FilterChips label="Worktrees shown">
          <FilterChip active={filter === "present"} count={all.filter(present).length} onClick={() => setFilter("present")}>On disk</FilterChip>
          <FilterChip active={filter === "all"} count={all.length} onClick={() => setFilter("all")}>All records</FilterChip>
        </FilterChips>
        <button className="btn btn-sm" disabled={measuring} onClick={async () => { setMeasuring(true); try { setSizes(await api.post<Record<string, number>>("/api/worktrees/sizes")); } catch (e) { toastError(e); } finally { setMeasuring(false); } }}>{measuring ? "Measuring…" : "Measure disk usage"}</button>
      </>}>
        {total !== null && <div className="text-[12px] text-ink-3">{size(total)} on disk</div>}
        {list.isLoading ? <Loading what="worktrees" /> : rows.length === 0 ? <Empty icon="branch">No task worktree yet.</Empty> : (
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
    </SettingsPage>
  );
}

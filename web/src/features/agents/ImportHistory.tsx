import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useRef, useState } from "react";
import type { HarnessKind } from "@/gen/HarnessKind";
import type { Session } from "@/gen/Session";
import { api } from "@/lib/api";
import { toastError } from "@/components/Toast";
import { HarnessMark, useHarnesses } from "@/components/HarnessMark";
import type { HistoryPage as Page } from "@/gen/HistoryPage";
export function ImportHistory({ onClose, onImported }: { onClose: () => void; onImported: (id: string) => void }) {
  const harnesses = useHarnesses().filter((h) => h.capabilities.history);
  const [harness, setHarness] = useState<HarnessKind | null>(null);
  const selectedHarness = harness ?? harnesses[0]?.id;
  const [archived, setArchived] = useState(false);
  const [cursor, setCursor] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [busy, setBusy] = useState(false);
  const client = useQueryClient();
  const q = useQuery({ queryKey: ["external-history",selectedHarness,archived,cursor], enabled: !!selectedHarness, queryFn: () => api.get<Page>(`/api/sessions/history?harness=${selectedHarness}&archived=${archived}${cursor ? `&cursor=${encodeURIComponent(cursor)}` : ""}`) });
  const rows = (q.data?.sessions ?? []).filter((s) => `${s.title} ${s.cwd}`.toLocaleLowerCase().includes(filter.toLocaleLowerCase()));
  const ref = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({ count: rows.length, getScrollElement: () => ref.current, estimateSize: () => 64, overscan: 4, useFlushSync: false });
  return <div role="dialog" aria-label="Import session history" className="border-b border-line bg-paper p-3">
    <div className="mb-2 flex flex-wrap items-center gap-2 text-xs"><b>Import session history</b>
      <select aria-label="History harness" className="field-input" value={selectedHarness} onChange={(e) => { setHarness(e.target.value as HarnessKind); setArchived(false); setCursor(null); }}>{harnesses.map((h) => <option key={h.id} value={h.id}>{h.name}</option>)}</select>
      {harnesses.find((h) => h.id === selectedHarness)?.capabilities.archived_history && <label className="flex items-center gap-1"><input type="checkbox" checked={archived} onChange={(e) => { setArchived(e.target.checked); setCursor(null); }} />Archived in native history</label>}
      <input aria-label="Filter external history" className="field-input" placeholder="Filter this page…" value={filter} onChange={(e) => setFilter(e.target.value)} />
      <button className="btn btn-sm" disabled={q.isFetching || busy} onClick={() => void q.refetch()}>Refresh</button>
      <button className="btn btn-sm ml-auto" onClick={onClose}>Close history</button>
    </div>
    <p className="mb-2 text-xs text-ink-3">Imports an archived conversation snapshot. Finish the session in its other client before resuming here.</p>
    {q.isPending && <p className="text-xs">Reading history…</p>}
    {q.error && <p className="text-xs text-danger">{q.error.message}</p>}
    {q.data && !rows.length && <p className="text-xs text-ink-3">No unimported conversations on this page.</p>}
    <div ref={ref} className="max-h-[230px] overflow-y-auto" style={{ height: Math.min(rows.length * 64,230) }}>
      <div style={{ height: v.getTotalSize(), position: "relative" }}>{v.getVirtualItems().map((r) => {
        const s = rows[r.index];
        return <div key={s.id} className="flex h-16 items-center gap-2 border-b border-line-2 px-2" style={{ position: "absolute", left:0,right:0,top:0,transform:`translateY(${r.start}px)` }}>
          <HarnessMark harness={s.harness} size={18} /><div className="min-w-0 flex-1 text-xs"><b className="block truncate" title={s.title}>{s.title}</b><span className="block truncate text-ink-3" title={s.cwd}>{s.cwd} · {new Date(s.started_at).toLocaleDateString()}</span></div>
          <button className="btn btn-sm" disabled={busy} onClick={async () => { setBusy(true); try { const record = await api.post<Session>("/api/sessions/history",{ harness:s.harness,id:s.id }); await client.invalidateQueries({ queryKey:["sessions"] }); await client.invalidateQueries({ queryKey:["external-history"] }); onImported(record.id); } catch(e) { toastError(e); } finally { setBusy(false); } }}>Import</button>
        </div>;
      })}</div>
    </div>
    <div className="mt-2 flex gap-2"><button className="btn btn-sm" disabled={!cursor || busy} onClick={() => setCursor(null)}>First page</button><button className="btn btn-sm" disabled={!q.data?.next || busy || q.isFetching} onClick={() => setCursor(q.data?.next ?? null)}>Next page</button></div>
  </div>;
}

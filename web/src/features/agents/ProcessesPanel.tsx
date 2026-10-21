import { memo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { SessionProcess } from "@/gen/SessionProcess";
import { api } from "@/lib/api";
import { toastError } from "@/components/Toast";

const ProcessRow = memo(function ProcessRow({ process, selected, select }: { process: SessionProcess; selected: boolean; select: () => void }) {
  return <button className={`h-12 w-full border-b border-line-2 px-3 text-left ${selected ? "bg-select" : "hover:bg-hover"}`} aria-pressed={selected} onClick={select} title={process.command}>
    <span className="block truncate font-mono text-xs">{process.command}</span>
    <span className="block truncate text-[11px] text-ink-3">Process {process.process_id}</span>
  </button>;
});

export function ProcessesPanel({ sessionId, processes, refresh }: { sessionId: string; processes: SessionProcess[]; refresh: () => Promise<unknown> }) {
  const [selected, select] = useState<string | null>(null);
  const [stopping, setStopping] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const key = (p: SessionProcess) => `${p.thread_id}/${p.process_id}`;
  const active = processes.find((p) => key(p) === selected) ?? processes[0];
  const list = useVirtualizer({ count: processes.length, getScrollElement: () => ref.current, estimateSize: () => 48, overscan: 3, useFlushSync: false });
  const stop = async () => {
    if (!active || stopping) return;
    setStopping(true);
    try {
      await api.post(`/api/sessions/${sessionId}/processes/stop`, { thread_id: active.thread_id, process_id: active.process_id });
      await refresh();
    } catch (e) { toastError(e); }
    finally { setStopping(false); }
  };
  return <div className="flex min-h-0 flex-1 flex-col">
    <p className="border-b border-line-2 px-3 py-2 text-xs text-ink-3">Background commands can outlive a failed turn. Stop them here, then Resume or Archive. Archive also stops these commands.</p>
    <div ref={ref} className="max-h-48 min-h-12 shrink-0 overflow-y-auto border-b border-line">
      <div style={{ height: list.getTotalSize(), position: "relative" }}>{list.getVirtualItems().map((it) => <div key={key(processes[it.index])} style={{ position: "absolute", top: 0, width: "100%", height: 48, transform: `translateY(${it.start}px)` }}>
        <ProcessRow process={processes[it.index]} selected={!!active && key(active) === key(processes[it.index])} select={() => select(key(processes[it.index]))} />
      </div>)}</div>
    </div>
    {active && <>
      <details key={key(active)} className="shrink-0 border-b border-line-2 px-3 text-xs">
        <summary className="cursor-pointer py-3.5">Full command</summary>
        <pre className="mb-2 max-h-32 overflow-auto whitespace-pre-wrap break-all font-mono">{active.command}</pre>
      </details>
      <div className="flex shrink-0 items-center gap-2 border-b border-line-2 px-3 py-2">
        <span className="min-w-0 flex-1 break-all text-xs text-ink-3">{active.cwd}</span>
        <button className="btn btn-sm shrink-0 text-danger" disabled={stopping} onClick={() => void stop()}>{stopping ? "Stopping…" : "Stop process"}</button>
      </div>
      <pre className="min-h-0 flex-1 overflow-auto whitespace-pre-wrap break-words p-3 font-mono text-xs">{active.output || "No output received yet."}</pre>
      <p className="px-3 py-1 text-[11px] text-ink-3">Recent output · full recorded output remains in Trace.</p>
    </>}
  </div>;
}

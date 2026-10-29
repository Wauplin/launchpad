import { memo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { Session } from "@/gen/Session";
import type { AgentTraceRef } from "@/gen/AgentTraceRef";
import { HarnessMark } from "@/components/HarnessMark";

const TraceRow = memo(function TraceRow({ session, selected, onToggle }: { session: Session; selected: boolean; onToggle: (trace: AgentTraceRef) => void }) {
  return <label className={`flex h-[48px] w-full cursor-pointer items-center gap-2 rounded-md px-2 text-left hover:bg-select ${selected ? "bg-select" : ""}`}>
    <input type="checkbox" checked={selected} onChange={() => onToggle({ session_id: session.id, through_seq: session.last_seq })} aria-label={session.title} />
    <HarnessMark harness={session.harness} size={15} />
    <span className="min-w-0 flex-1">
      <span className="block truncate text-xs font-semibold">{session.title}</span>
      <span className="block truncate text-[11px] text-ink-3">{session.status === "closed" ? "Archived" : session.status} · {new Date(session.started_at).toLocaleString()} · {session.cwd}</span>
    </span>
  </label>;
});

export function AgentTracePicker({ sessions, selected, onToggle }: { sessions: Session[]; selected: AgentTraceRef[]; onToggle: (trace: AgentTraceRef) => void }) {
  const [search, setSearch] = useState("");
  const scroll = useRef<HTMLDivElement>(null);
  const rows = sessions.filter((s) => s.last_seq > 0 && `${s.title} ${s.cwd}`.toLowerCase().includes(search.toLowerCase())).sort((a, b) => b.started_at - a.started_at);
  const virtual = useVirtualizer({ count: rows.length, getScrollElement: () => scroll.current, estimateSize: () => 48, overscan: 4, useFlushSync: false });
  return <div className="grid gap-1">
    <input className="field-input w-full" aria-label="Find agent trace" placeholder="Find an agent by title or folder" value={search} onChange={(e) => { setSearch(e.target.value); scroll.current?.scrollTo({ top: 0 }); }} />
    <p className="px-2 text-[11px] text-ink-3">Select traces to attach, including messages and tool results.</p>
    <div ref={scroll} className="max-h-[240px] overflow-y-auto" style={{ height: Math.min(rows.length * 48, 240) }}>
      <div className="relative" style={{ height: virtual.getTotalSize() }}>
        {virtual.getVirtualItems().map((row) => <div key={rows[row.index].id} className="absolute left-0 top-0 w-full" style={{ transform: `translateY(${row.start}px)`, height: row.size }}><TraceRow session={rows[row.index]} selected={selected.some((t) => t.session_id === rows[row.index].id)} onToggle={onToggle} /></div>)}
      </div>
    </div>
    {!rows.length && <p className="px-2 py-2 text-xs text-ink-3">No matching agent traces in this workspace.</p>}
  </div>;
}

import { memo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Link } from "@tanstack/react-router";
import { SessionStatus } from "@/features/agents/SessionBits";
import { usePhone } from "@/lib/phone";
import type { TopicContext } from "@/gen/TopicContext";
import type { Session } from "@/gen/Session";
import { useTopics } from "./data";
import { useSessions } from "@/lib/queries";
import { useScope } from "@/lib/workspace";

const SourceRow = memo(function SourceRow({ session, cutoff }: { session: Session; cutoff: number }) {
  const phone = usePhone();
  return <div className="grid h-full content-center gap-1 border-b border-line-2 py-2"><Link to="/agents" search={{ session: session.id }} style={phone ? { minHeight: 44, display: "flex", alignItems: "center" } : undefined} className="truncate text-teal-ink hover:underline">{session.title}</Link><span><SessionStatus s={session} />{cutoff === 0 ? " · No stored history yet" : ` · Through event ${cutoff}`}</span></div>;
});
function TopicChip({ topic, onRemove }: { topic: TopicContext; onRemove: (id: string) => void }) {
  const phone = usePhone();
  const all = useTopics().data ?? [];
  const sessions = useSessions().data ?? [];
  const scope = useScope();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const ids = topic.snapshot?.map(t => t.session_id) ?? all.find(t => t.id === topic.id)?.session_ids ?? [];
  const runs = sessions.filter(s => ids.includes(s.id) && scope.hasSession(s));
  const virtual = useVirtualizer({ count: runs.length, getScrollElement: () => ref.current, estimateSize: () => phone ? 88 : 68, overscan: 3, enabled: open, useFlushSync: false });
  return <span className="inline-flex max-w-full items-start rounded-control border border-line bg-chrome text-[12px]">
    <details onToggle={e => setOpen(e.currentTarget.open)} className="min-w-0 px-2 py-1">
      <summary style={phone ? { minHeight: 44, display: "flex", alignItems: "center" } : undefined} className="cursor-pointer truncate text-teal-ink">{topic.name} · {runs.length} sessions</summary>
      <div className="py-2 text-ink-2">
        <div ref={ref} className="max-h-48 overflow-auto"><div style={{ height: virtual.getTotalSize(), position: "relative" }}>{virtual.getVirtualItems().map(v => { const s = runs[v.index]; return <div key={s.id} style={{ position: "absolute", top: v.start, left: 0, width: "100%", height: v.size }}><SourceRow session={s} cutoff={topic.snapshot?.find(t => t.session_id === s.id)?.through_seq ?? s.last_seq} /></div>; })}</div></div>
        {topic.notes?.map((note, i) => <div key={i}>{note}</div>)}
        {runs.length === 0 && <span>No available history yet.</span>}
        {runs.length < ids.length && <div>Some history is unavailable.</div>}
      </div>
    </details>
    <button className="iconbtn shrink-0" aria-label={`Remove topic context ${topic.name}`} onClick={() => onRemove(topic.id)}>×</button>
  </span>;
}
export function TopicContextChips({ topics = [], onRemove }: { topics?: TopicContext[]; onRemove: (id: string) => void }) {
  return topics.map(topic => <TopicChip key={topic.id} topic={topic} onRemove={onRemove} />);
}

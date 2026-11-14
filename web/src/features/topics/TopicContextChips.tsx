import { memo, useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Link } from "@tanstack/react-router";
import { SessionStatus } from "@/features/agents/SessionBits";
import { Popover } from "@/components/Popover";
import { Icon } from "@/components/Icon";
import { usePhone } from "@/lib/phone";
import { ago } from "@/lib/format";
import type { TopicContext } from "@/gen/TopicContext";
import type { Session } from "@/gen/Session";
import { useTopics } from "./data";
import { useSessions } from "@/lib/queries";
import { useScope } from "@/lib/workspace";

const SourceRow = memo(function SourceRow({ session, cutoff, close }: { session: Session; cutoff: number; close: () => void }) {
  return <Link to="/agents" search={{ session: session.id }} onClick={close} title={cutoff === 0 ? "No stored history yet" : `Its trace through event ${cutoff}`} className="flex h-full min-w-0 items-center gap-2 rounded-control px-2 text-[13px] text-ink no-underline hover:bg-chrome">
    <span className="min-w-0 flex-1 truncate">{session.title}</span>
    <span className="flex shrink-0 items-center gap-1.5 text-[12px] text-ink-3"><SessionStatus s={session} />{ago(session.status_since)}</span>
  </Link>;
});

/** The agents whose traces a topic passes on, as of now or as frozen at dispatch, with the notes Home added. */
function Sources({ topic, runs, missing, close }: { topic: TopicContext; runs: Session[]; missing: boolean; close: () => void }) {
  const phone = usePhone();
  const ref = useRef<HTMLDivElement>(null);
  const virtual = useVirtualizer({ count: runs.length, getScrollElement: () => ref.current, estimateSize: () => phone ? 44 : 30, overscan: 4, useFlushSync: false });
  return <div className="grid gap-1 p-1 text-[12px] text-ink-3">
    <span className="px-2">{topic.snapshot ? "The agent reads these traces:" : "The agent will read the traces of:"}</span>
    {runs.length > 0 && <div ref={ref} className="max-h-64 overflow-y-auto"><div style={{ height: virtual.getTotalSize(), position: "relative" }}>{virtual.getVirtualItems().map((v) => { const s = runs[v.index]; return <div key={s.id} style={{ position: "absolute", top: v.start, left: 0, width: "100%", height: v.size }}><SourceRow session={s} close={close} cutoff={topic.snapshot?.find((t) => t.session_id === s.id)?.through_seq ?? s.last_seq} /></div>; })}</div></div>}
    {runs.length === 0 && <span className="px-2">No stored history yet.</span>}
    {missing && <span className="px-2">Some history is unavailable.</span>}
    {topic.notes?.map((note, i) => <span key={i} className="px-2">{note}</span>)}
  </div>;
}

function TopicChip({ topic, onRemove }: { topic: TopicContext; onRemove: (id: string) => void }) {
  const all = useTopics().data ?? [];
  const sessions = useSessions().data ?? [];
  const scope = useScope();
  const ids = topic.snapshot?.map((t) => t.session_id) ?? all.find((t) => t.id === topic.id)?.session_ids ?? [];
  const runs = sessions.filter((s) => ids.includes(s.id) && scope.hasSession(s));
  return <span className="inline-flex h-7 min-w-0 max-w-full items-center rounded-control border border-line bg-chrome text-[12px] text-ink-2">
    <Popover viewport width={320} label={`Topic ${topic.name}`} className="flex min-w-0" trigger={({ open, toggle }) => <button className="flex min-w-0 items-center gap-1.5 pl-2" aria-expanded={open} title={`Topic: the traces of ${runs.length} agent${runs.length === 1 ? "" : "s"}`} onClick={toggle}>
      <Icon name="tag" size={12} className="shrink-0" /><span className="truncate">{topic.name}</span><span className="shrink-0 text-ink-3 tabular-nums">{runs.length}</span>
    </button>}>
      {(close) => <Sources topic={topic} runs={runs} missing={runs.length < ids.length} close={close} />}
    </Popover>
    <button className="iconbtn shrink-0" aria-label={`Remove topic ${topic.name}`} title="Remove" onClick={() => onRemove(topic.id)}><Icon name="x" size={12} /></button>
  </span>;
}

export function TopicContextChips({ topics = [], onRemove }: { topics?: TopicContext[]; onRemove: (id: string) => void }) {
  return topics.map((topic) => <TopicChip key={topic.id} topic={topic} onRemove={onRemove} />);
}

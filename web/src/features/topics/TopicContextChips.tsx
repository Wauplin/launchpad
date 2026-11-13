import { Link } from "@tanstack/react-router";
import { SessionStatus } from "@/features/agents/SessionBits";
import { usePhone } from "@/lib/phone";
import type { TopicContext } from "@/gen/TopicContext";
import { useTopics } from "./data";
import { useSessions } from "@/lib/queries";
import { useScope } from "@/lib/workspace";
export function TopicContextChips({ topics = [], onRemove }: {topics?: TopicContext[]; onRemove: (id:string) => void}) {
  const phone = usePhone();
  const all = useTopics().data ?? []; const sessions = useSessions().data ?? []; const scope = useScope();
  return topics.map(topic => {
    const ids = topic.snapshot?.map(t => t.session_id) ?? all.find(t => t.id === topic.id)?.session_ids ?? [];
    const runs = sessions.filter(s => ids.includes(s.id) && scope.hasSession(s));
    return <span key={topic.id} className="inline-flex max-w-full items-start rounded-control border border-line bg-chrome text-[12px]">
      <details className="min-w-0 px-2 py-1"><summary style={phone ? {minHeight:44,display:"flex",alignItems:"center"} : undefined} className="cursor-pointer truncate text-teal-ink">{topic.name} · {runs.length} sessions</summary><div className="max-h-48 overflow-auto py-2 text-ink-2">{runs.map(s => <div key={s.id} className="grid gap-1 border-b border-line-2 py-2"><Link to="/agents" search={{session:s.id}} className="text-teal-ink hover:underline">{s.title}</Link><span><SessionStatus s={s} />{s.last_seq === 0 ? " · No stored history yet" : ` · Through event ${topic.snapshot?.find(t => t.session_id === s.id)?.through_seq ?? s.last_seq}`}</span></div>)}{topic.notes?.map((note,i) => <div key={i}>{note}</div>)}{runs.length === 0 && <span>No available history yet.</span>}{runs.length < ids.length && <div>Some history is unavailable.</div>}</div></details>
      <button className="iconbtn shrink-0" aria-label={`Remove topic context ${topic.name}`} onClick={() => onRemove(topic.id)}>×</button>
    </span>;
  });
}

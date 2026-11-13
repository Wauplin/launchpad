import { useComposer } from "@/stores/composer";
import { SessionStatus } from "@/features/agents/SessionBits";
import { memo, useRef, useState } from "react";
import { Link, useNavigate, useParams } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { PageHeader, Empty, Loading, ErrorState, MoreMenu } from "@/components/ui";
import { MenuItem } from "@/components/Popover";
import { PhoneHeader } from "@/mobile/PhoneHeader";
import { phoneBack } from "@/mobile/back";
import { usePhone } from "@/lib/phone";
import { useScope } from "@/lib/workspace";
import { useSessions } from "@/lib/queries";
import { runAction } from "@/lib/actions";
import { ActionMenuItem } from "@/components/ActionButton";
import { toastError } from "@/components/Toast";
import { ago } from "@/lib/format";
import { TopicPicker } from "./TopicPicker";
import { useTopics, useTopicMutations, type Topic } from "./data";
import type { Session } from "@/gen/Session";

export function TopicsView() { return <TopicPage />; }
export function TopicDetail() { const { id } = useParams({ strict: false }); return <TopicPage id={id} />; }
const TopicRow = memo(function TopicRow({ topic }: { topic: Topic }) {
  return <Link to="/topics/$id" params={{ id: topic.id }} className="flex h-full items-center justify-between gap-3 border-b border-line-2 px-3 text-[13px] hover:bg-chrome"><span className="truncate font-medium">{topic.name}</span><span className="shrink-0 text-[12px] text-ink-3">{topic.session_ids.length} sessions</span></Link>;
});
const RunRow = memo(function RunRow({ session }: { session: Session }) {
  return <Link to="/agents" search={{ session: session.id }} className="flex h-full flex-col justify-center gap-1 border-b border-line-2 px-3 hover:bg-chrome"><span className="truncate text-[13px] font-medium">{session.title}</span><span className="flex items-center gap-2 text-[12px] text-ink-3"><SessionStatus s={session} />{ago(session.started_at)}</span></Link>;
});
function TopicPage({ id }: { id?: string }) {
  const query = useTopics();
  const { refresh } = useTopicMutations();
  const phone = usePhone();
  const scope = useScope();
  const navigate = useNavigate();
  const sessions = (useSessions().data ?? []).filter(scope.hasSession);
  const topic = query.data?.find((t) => t.id === id);
  const [search, setSearch] = useState("");
  const [rename, setRename] = useState<string | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  const needle = search.trim().toLocaleLowerCase();
  const rows = id ? sessions.filter((s) => topic?.session_ids.includes(s.id) && (!needle || `${s.title} ${s.prompt}`.toLocaleLowerCase().includes(needle))).sort((a,b) => b.started_at - a.started_at) : (query.data ?? []).filter((t) => t.name.toLocaleLowerCase().includes(needle));
  const virtual = useVirtualizer({ count: rows.length, getScrollElement: () => ref.current, estimateSize: () => id ? 64 : 52, overscan: 6, useFlushSync: false });
  const title = id ? topic?.name ?? "Topic" : "Topics";
  const launch = (summary = false) => topic && useComposer.getState().start({ topics: [{ id: topic.id, name: topic.name }], ...(summary ? {text: "Read every included trace from this topic and summarize its decisions, implementation and remaining work. Cite each source with its /agents?session=<id> link and report unavailable or unfinished history."} : {}) });
  const actions = id && topic ? <MoreMenu label="Topic actions">{(close) => <><MenuItem label="Summarize…" onClick={() => { launch(true); close(); }} /><MenuItem label="Rename" onClick={() => { setRename(topic.name); close(); }} /><ActionMenuItem name="topic.delete" params={{ id: topic.id }} close={close} onDone={async () => { await refresh(); void navigate({ to: "/topics" }); }} /></>}</MoreMenu> : <TopicPicker label="New topic" onPick={async (t) => { await navigate({ to: "/topics/$id", params: { id: t.id } }); }} />;
  return <div className="flex h-full min-h-0 flex-col">
    {phone ? <PhoneHeader title={title} onBack={id ? () => phoneBack("/topics") : undefined} primary={id && topic ? <button data-tour="topic.launch" className="btn btn-sm btn-primary" onClick={() => launch()}>Launch</button> : actions} actions={id ? actions : undefined} /> : <PageHeader title={title} meta={`${rows.length} ${id ? "sessions" : "topics"}`} intro={id ? undefined : "Collect related agent sessions across folders, then pass their history to the next agent."} actions={id && topic ? <div className="flex items-center gap-2"><button data-tour="topic.launch" className="btn btn-sm btn-primary" onClick={() => launch()}>Launch</button>{actions}</div> : actions} />}
    <div className="flex min-h-0 w-full max-w-page flex-1 flex-col gap-3 px-5 py-4">
      {id && !phone && <Link to="/topics" className="text-[12px] text-teal-ink">All topics</Link>}
      {rename !== null && topic && <form className="flex gap-2" onSubmit={(e) => { e.preventDefault(); void runAction("topic.rename", { id: topic.id }, { name: rename }).then(async () => { await refresh(); setRename(null); }).catch(toastError); }}><input className="field-input min-w-0 flex-1" aria-label="Topic name" maxLength={80} value={rename} onChange={(e) => setRename(e.target.value)} /><button className="btn btn-sm" disabled={!rename.trim()}>Save</button><button type="button" className="btn btn-sm" onClick={() => setRename(null)}>Cancel</button></form>}
      <div className="flex flex-wrap items-center gap-2"><input className="field-input min-w-0 flex-1" aria-label={id ? "Search topic sessions" : "Search topics"} placeholder={id ? "Search sessions…" : "Search topics…"} value={search} onChange={(e) => setSearch(e.target.value)} />{id && topic && <Link to="/agents" search={{}} className="btn btn-sm">Add sessions</Link>}</div>
      {query.isPending ? <Loading /> : query.error ? <ErrorState error={query.error} /> : id && !topic ? <Empty>Topic no longer exists.</Empty> : rows.length === 0 ? <Empty>{needle ? "No matches." : id ? "No sessions yet. Add sessions from Agents or from a session’s Add topic button." : "No topics yet. Create a topic with just a name."}</Empty> : null}
      <div ref={ref} className="min-h-0 flex-1 overflow-auto"><div style={{ height: virtual.getTotalSize(), position: "relative" }}>{virtual.getVirtualItems().map((v) => <div key={v.key} style={{ position: "absolute", top: v.start, left: 0, width: "100%", height: v.size }}>{id ? <RunRow session={rows[v.index] as Session} /> : <TopicRow topic={rows[v.index] as Topic} />}</div>)}</div></div>
    </div>
  </div>;
}

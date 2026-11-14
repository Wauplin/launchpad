import { useComposer } from "@/stores/composer";
import { SessionStatus } from "@/features/agents/SessionBits";
import { memo, useRef, useState } from "react";
import { Link, useNavigate, useParams } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { PageHeader, Empty, Loading, ErrorState, MoreMenu, RowActions } from "@/components/ui";
import { MenuItem } from "@/components/Popover";
import { Icon } from "@/components/Icon";
import { PhoneHeader } from "@/mobile/PhoneHeader";
import { phoneBack } from "@/mobile/back";
import { usePhone } from "@/lib/phone";
import { useScope } from "@/lib/workspace";
import { projectName, useProjects, useSessions } from "@/lib/queries";
import { runAction } from "@/lib/actions";
import { ActionMenuItem } from "@/components/ActionButton";
import { toast, toastError } from "@/components/Toast";
import { ago } from "@/lib/format";
import { TopicName, TopicPicker } from "./TopicPicker";
import { useTopics, useTopicMutations, type Topic } from "./data";
import type { Session } from "@/gen/Session";

export function TopicsView() { return <TopicPage />; }
export function TopicDetail() { const { id } = useParams({ strict: false }); return <TopicPage id={id} />; }

const SUMMARY = "Read every included trace from this topic and summarize its decisions, implementation and remaining work. Cite each source with its /agents?session=<id> link and report unavailable or unfinished history.";

const TopicRow = memo(function TopicRow({ topic, last }: { topic: Topic; last?: number }) {
  return <Link to="/topics/$id" params={{ id: topic.id }} className="flex h-full min-w-0 items-center gap-2 border-b border-line-2 px-3 text-[13px] text-ink no-underline hover:bg-chrome">
    <Icon name="tag" size={12} className="shrink-0 text-ink-3" />
    <span className="flex min-w-0 flex-1 gap-1 font-medium"><TopicName topic={topic} /></span>
    <span className="shrink-0 text-[12px] text-ink-3 tabular-nums">{topic.session_ids.length} session{topic.session_ids.length === 1 ? "" : "s"}{last ? ` · ${ago(last)}` : ""}</span>
  </Link>;
});

const RunRow = memo(function RunRow({ session, folder, onRemove }: { session: Session; folder: string; onRemove: (s: Session) => void }) {
  return <div className="group flex h-full min-w-0 items-center gap-2 border-b border-line-2 pr-1">
    <Link to="/agents" search={{ session: session.id }} className="flex h-full min-w-0 flex-1 flex-col justify-center gap-0.5 px-3 text-ink no-underline hover:bg-chrome">
      <span className="truncate text-[13px] font-medium">{session.title}</span>
      <span className="flex min-w-0 items-center gap-2 text-[12px] text-ink-3"><SessionStatus s={session} /><span className="truncate">{folder}</span><span className="shrink-0">{ago(session.status_since)}</span></span>
    </Link>
    <RowActions label="Session actions" width={0} more={(close) => <MenuItem icon={<Icon name="x" size={14} />} label="Remove from this topic" detail="The session stays; Undo brings it back" onClick={() => { close(); onRemove(session); }} />} />
  </div>;
});

function TopicPage({ id }: { id?: string }) {
  const query = useTopics();
  const { refresh, add } = useTopicMutations();
  const phone = usePhone();
  const scope = useScope();
  const navigate = useNavigate();
  const projects = useProjects().data;
  const sessions = (useSessions().data ?? []).filter(scope.hasSession);
  const topic = query.data?.find((t) => t.id === id);
  const [search, setSearch] = useState("");
  const [rename, setRename] = useState<string | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  const needle = search.trim().toLocaleLowerCase();
  const byId = new Map(sessions.map((s) => [s.id, s]));
  const lastOf = (t: Topic) => Math.max(0, ...t.session_ids.map((sid) => byId.get(sid)?.status_since ?? 0));
  const members = id ? sessions.filter((s) => topic?.session_ids.includes(s.id)).sort((a, b) => b.status_since - a.status_since) : [];
  const runs = members.filter((s) => !needle || `${s.title} ${s.prompt}`.toLocaleLowerCase().includes(needle));
  const topics = (query.data ?? []).filter((t) => t.name.toLocaleLowerCase().includes(needle)).sort((a, b) => lastOf(b) - lastOf(a));
  const count = id ? runs.length : topics.length;
  const virtual = useVirtualizer({ count, getScrollElement: () => ref.current, estimateSize: () => id ? 52 : phone ? 48 : 36, overscan: 6, useFlushSync: false });
  const launch = (summary = false) => topic && useComposer.getState().start({ topics: [{ id: topic.id, name: topic.name }], ...(summary ? { text: SUMMARY } : {}) });
  const remove = async (s: Session) => {
    if (!topic) return;
    try { await add(topic.id, [s.id], false); toast(`Removed from ${topic.name}`, { label: "Undo", run: () => { void add(topic.id, [s.id]).catch(toastError); } }); } catch (e) { toastError(e); }
  };
  const menu = (close: () => void) => topic && <>
    <MenuItem icon={<Icon name="spark" size={14} />} label="Summarize…" detail="Launch an agent that reads every trace" onClick={() => { close(); launch(true); }} />
    <MenuItem icon={<Icon name="agents" size={14} />} label="Show in Agents" detail="The Agents list, filtered by this topic" onClick={() => { close(); void navigate({ to: "/agents", search: { topic: topic.id } }); }} />
    <MenuItem icon={<Icon name="pencil" size={14} />} label="Rename" onClick={() => { close(); setRename(topic.name); }} />
    <ActionMenuItem name="topic.delete" params={{ id: topic.id }} close={close} onDone={async () => { await refresh(); void navigate({ to: "/topics" }); }} />
  </>;
  const primary = topic && <button data-tour="topic.launch" className="btn btn-sm btn-primary" title="Launch an agent with the traces of this topic as context" onClick={() => launch()}>Launch</button>;
  const create = <TopicPicker label={<><Icon name="plus" size={12} />New topic</>} title="New topic" onPick={async (t) => { await navigate({ to: "/topics/$id", params: { id: t.id } }); }} />;
  const title = id ? topic ? <span className="flex min-w-0 gap-1"><TopicName topic={topic} /></span> : "Topic" : "Topics";
  const last = topic && lastOf(topic);
  const meta = id
    ? <>{!phone && <Link to="/topics" className="text-teal-ink">Topics</Link>}<span>{members.length} session{members.length === 1 ? "" : "s"}</span>{last ? <span>active {ago(last)}</span> : null}</>
    : <span>{query.data?.length ?? 0} topics</span>;
  const empty = id
    ? needle ? "No session matches." : "No sessions yet. Add one from its meta line in Agents, or with Select in the Agents list."
    : needle ? "No topic matches." : "No topics yet. Launching an agent with another agent's trace creates one, or add a session to a new topic from its meta line.";
  return <div className="flex h-full min-h-0 flex-col">
    {phone
      ? <PhoneHeader title={title} onBack={id ? () => phoneBack("/topics") : undefined} subtitle={id ? meta : undefined} primary={id ? primary : create} actions={id && topic ? menu : undefined} />
      : <PageHeader title={title} meta={meta} intro={id ? undefined : "Topics group related agent sessions across folders. A new agent launched with another agent's trace joins its topics, or gets a new named topic together with it. Attach a topic to pass its history to the next agent."} actions={id ? topic && <>{primary}<MoreMenu label="Topic actions">{(close) => menu(close)}</MoreMenu></> : create} />}
    <div className="flex min-h-0 w-full max-w-page flex-1 flex-col gap-2 px-5 py-3">
      {rename !== null && topic && <form className="flex gap-2" onSubmit={(e) => { e.preventDefault(); void runAction("topic.rename", { id: topic.id }, { name: rename }).then(async () => { await refresh(); setRename(null); }).catch(toastError); }}>
        <input autoFocus className="field-input min-w-0 flex-1" aria-label="Topic name" maxLength={80} value={rename} onChange={(e) => setRename(e.target.value)} onKeyDown={(e) => { if (e.key === "Escape") setRename(null); }} />
        <button className="btn btn-sm" disabled={!rename.trim()}>Save</button><button type="button" className="btn btn-sm" onClick={() => setRename(null)}>Cancel</button>
      </form>}
      {(id ? members.length : query.data?.length ?? 0) > 6 && <input className="field-input w-full" aria-label={id ? "Search topic sessions" : "Search topics"} placeholder={id ? "Search sessions…" : "Search topics…"} value={search} onChange={(e) => setSearch(e.target.value)} />}
      {query.isPending ? <Loading /> : query.error ? <ErrorState error={query.error} /> : id && !topic ? <Empty>This topic no longer exists.</Empty> : count === 0 ? <Empty>{empty}</Empty> : null}
      <div ref={ref} className="min-h-0 flex-1 overflow-auto"><div style={{ height: virtual.getTotalSize(), position: "relative" }}>{virtual.getVirtualItems().map((v) => <div key={v.key} style={{ position: "absolute", top: v.start, left: 0, width: "100%", height: v.size }}>
        {id ? <RunRow session={runs[v.index]} folder={projectName(projects, runs[v.index].project_id, runs[v.index].cwd)} onRemove={remove} /> : <TopicRow topic={topics[v.index]} last={lastOf(topics[v.index])} />}
      </div>)}</div></div>
    </div>
  </div>;
}

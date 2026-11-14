import { useVirtualizer } from "@tanstack/react-virtual";
import { useRef, useState, type ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import { Popover } from "@/components/Popover";
import { Icon } from "@/components/Icon";
import { Sheet } from "@/mobile/Sheet";
import { usePhone } from "@/lib/phone";
import { useScope } from "@/lib/workspace";
import { toast, toastError } from "@/components/Toast";
import { useTopics, useTopicMutations, type Topic } from "./data";

/** A topic's name, with a quiet "naming…" while Home waits for the name generated at the launch that created it. */
export function TopicName({ topic }: { topic: Pick<Topic, "name" | "naming"> }) {
  return <><span className="truncate">{topic.name}</span>{topic.naming && <span className="shrink-0 text-ink-3" title="A name is being generated from the agents' work">· naming…</span>}</>;
}

/** A button that opens the topic choices: a popover on the laptop, a sheet on the phone. */
export function TopicPicker({ onPick, exclude = [], label = "Add to topic…", title = "Add to topic", button = "btn btn-sm" }: { onPick: (topic: Topic) => Promise<void> | void; exclude?: string[]; label?: ReactNode; title?: string; button?: string }) {
  const phone = usePhone();
  const [open, setOpen] = useState(false);
  const content = (close: () => void) => <TopicChoices onPick={onPick} exclude={exclude} close={close} />;
  if (phone) return <><button className={button} onClick={() => setOpen(true)}>{label}</button>{open && <Sheet title={title} onClose={() => setOpen(false)}>{content(() => setOpen(false))}</Sheet>}</>;
  return <Popover viewport width={300} label={title} trigger={({ toggle }) => <button className={button} onClick={toggle}>{label}</button>}>{content}</Popover>;
}

/** Find a topic or create one by typing its name; Enter picks the first line. */
export function TopicChoices({ onPick, exclude, close }: { onPick: (topic: Topic) => Promise<void> | void; exclude: string[]; close: () => void }) {
  const query = useTopics();
  const { workspace } = useScope();
  const { create } = useTopicMutations();
  const [search, setSearch] = useState("");
  const [busy, setBusy] = useState(false);
  const topics = query.data ?? [];
  const ref = useRef<HTMLDivElement>(null);
  const phone = usePhone();
  const name = search.trim();
  const needle = name.toLocaleLowerCase();
  const matches = topics.filter((t) => !exclude.includes(t.id) && t.name.toLocaleLowerCase().includes(needle));
  const canCreate = !!name && !!query.data && !topics.some((t) => t.name.toLocaleLowerCase() === needle);
  const rows: (Topic | null)[] = [...(canCreate ? [null] : []), ...matches];
  const virtual = useVirtualizer({ count: rows.length, getScrollElement: () => ref.current, estimateSize: () => phone ? 44 : 30, overscan: 4, useFlushSync: false });
  const pick = async (topic: Topic | null) => {
    if (busy) return;
    setBusy(true);
    try { await onPick(topic ?? await create(workspace.id, name)); close(); } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  return <div className="grid gap-1.5 p-2">
    <input autoFocus className="field-input w-full" aria-label="Find or create a topic" placeholder="Find or create a topic…" maxLength={80} value={search} onChange={(e) => setSearch(e.target.value)}
      onKeyDown={(e) => { if (e.key === "Enter" && rows.length) { e.preventDefault(); void pick(rows[0]); } }} />
    {rows.length > 0 && <div ref={ref} role="listbox" aria-label="Topics" className="max-h-64 overflow-y-auto"><div style={{ height: virtual.getTotalSize(), position: "relative" }}>{virtual.getVirtualItems().map((v) => {
      const t = rows[v.index];
      return <button key={t?.id ?? "create"} role="option" aria-selected={v.index === 0} style={{ position: "absolute", top: v.start, height: v.size, left: 0 }} className={`flex w-full items-center gap-2 rounded-control px-2 text-left text-[13px] hover:bg-chrome ${v.index === 0 && name ? "bg-chrome" : ""}`} disabled={busy} onClick={() => void pick(t)}>
        <Icon name={t ? "tag" : "plus"} size={12} className="shrink-0 text-ink-3" />
        {t ? <><span className="flex min-w-0 flex-1 gap-1"><TopicName topic={t} /></span><span className="shrink-0 text-[12px] text-ink-3 tabular-nums">{t.session_ids.length}</span></> : <span className="min-w-0 flex-1 truncate">Create “{name}”</span>}
      </button>;
    })}</div></div>}
    {query.isPending && <span className="px-2 text-[12px] text-ink-3">Loading topics…</span>}
    {query.error && <span className="px-2 text-[12px] text-danger-ink">{query.error.message}</span>}
    {query.data && rows.length === 0 && <span className="px-2 text-[12px] text-ink-3">{name ? "Already in this topic." : topics.length ? "No other topic yet. Type a name to create one." : "Type a name to create your first topic."}</span>}
  </div>;
}

/** The topics of one session, then the choices to add it to another. */
function SessionTopicsPanel({ sessionId, topics, close }: { sessionId: string; topics: Topic[]; close: () => void }) {
  const { add } = useTopicMutations();
  const remove = async (t: Topic) => {
    try { await add(t.id, [sessionId], false); toast(`Removed from ${t.name}`, { label: "Undo", run: () => { void add(t.id, [sessionId]).catch(toastError); } }); } catch (e) { toastError(e); }
  };
  return <div className="grid">
    {topics.length > 0 && <ul className="m-0 grid list-none border-b border-line-2 p-1">
      {topics.map((t) => <li key={t.id} className="flex min-w-0 items-center gap-1">
        <Link to="/topics/$id" params={{ id: t.id }} onClick={close} className="flex min-h-7 min-w-0 flex-1 items-center gap-2 rounded-control px-2 text-[13px] text-ink no-underline hover:bg-chrome">
          <Icon name="tag" size={12} className="shrink-0 text-teal-ink" /><span className="flex min-w-0 flex-1 gap-1"><TopicName topic={t} /></span><span className="shrink-0 text-[12px] text-ink-3 tabular-nums">{t.session_ids.length}</span>
        </Link>
        <button className="iconbtn shrink-0 text-ink-3" title={`Remove from ${t.name}`} aria-label={`Remove from ${t.name}`} onClick={() => void remove(t)}><Icon name="x" size={14} /></button>
      </li>)}
    </ul>}
    <TopicChoices exclude={topics.map((t) => t.id)} close={close} onPick={(t) => add(t.id, [sessionId])} />
  </div>;
}

/**
 * A session's topics in its meta line: one quiet chip with the first topic's name (+N for more), which opens the
 * list with links and Remove, and the field to add another. Without a topic, the laptop shows "Topic" there and
 * the phone shows nothing (⋯ → Topics… opens the same sheet through `open`).
 */
export function SessionTopics({ sessionId, open = false, onClose }: { sessionId: string; open?: boolean; onClose?: () => void }) {
  const phone = usePhone();
  const { data = [] } = useTopics();
  const [own, setOwn] = useState(false);
  const topics = data.filter((t) => t.session_ids.includes(sessionId));
  const first = topics[0];
  const label = first ? <><span className="flex min-w-0 gap-1"><TopicName topic={first} /></span>{topics.length > 1 && <span className="shrink-0">+{topics.length - 1}</span>}</> : <span>Topic</span>;
  const describe = first ? `Topics: ${topics.map((t) => t.name).join(", ")}` : "Add this session to a topic";
  const chip = (toggle: () => void, expanded: boolean) => <button data-tour="session.topics" className={`inline-flex h-5 min-w-0 max-w-56 shrink-0 items-center gap-1 rounded-control px-1 text-[12px] hover:bg-chrome hover:text-ink ${first ? "text-teal-ink" : "text-ink-3"}`} title={describe} aria-label={describe} aria-haspopup="dialog" aria-expanded={expanded} onClick={toggle}>
    <Icon name={first ? "tag" : "plus"} size={12} className="shrink-0" />{label}
  </button>;
  if (phone) {
    const shown = own || open;
    const close = () => { setOwn(false); onClose?.(); };
    return <>
      {first && chip(() => setOwn(true), shown)}
      {shown && <Sheet title="Topics" onClose={close}><SessionTopicsPanel sessionId={sessionId} topics={topics} close={close} /></Sheet>}
    </>;
  }
  return <Popover viewport width={300} label="Topics" className="flex min-w-0 shrink-0" trigger={({ open, toggle }) => chip(toggle, open)}>
    {(close) => <SessionTopicsPanel sessionId={sessionId} topics={topics} close={close} />}
  </Popover>;
}

/** The Agents list's topic filter: a chip named after the chosen topic, with the workspace's topics to pick from. */
export function TopicFilterChip({ value, onChange }: { value?: string; onChange: (id?: string) => void }) {
  const { data = [] } = useTopics();
  const phone = usePhone();
  const [open, setOpen] = useState(false);
  const active = data.find((t) => t.id === value);
  const chip = (toggle: () => void) => <button type="button" className="lp-chip max-w-48" aria-pressed={!!active} aria-haspopup="dialog" title={active ? `Only sessions of ${active.name}` : "Show the sessions of one topic"} onClick={toggle}>
    <Icon name="tag" size={12} className="shrink-0" /><span className="truncate">{active?.name ?? "Topic"}</span><Icon name="chev" size={12} className="shrink-0" />
  </button>;
  const list = (close: () => void) => <TopicFilterList value={value} topics={data} close={close} onChange={onChange} />;
  if (phone) return <>{chip(() => setOpen(true))}{open && <Sheet title="Filter by topic" onClose={() => setOpen(false)}>{list(() => setOpen(false))}</Sheet>}</>;
  return <Popover viewport width={260} label="Filter by topic" className="flex min-w-0" trigger={({ toggle }) => chip(toggle)}>{list}</Popover>;
}

function TopicFilterList({ value, topics, close, onChange }: { value?: string; topics: Topic[]; close: () => void; onChange: (id?: string) => void }) {
  const phone = usePhone();
  const ref = useRef<HTMLDivElement>(null);
  const [search, setSearch] = useState("");
  const needle = search.trim().toLocaleLowerCase();
  const rows = topics.filter((t) => t.name.toLocaleLowerCase().includes(needle));
  const virtual = useVirtualizer({ count: rows.length, getScrollElement: () => ref.current, estimateSize: () => phone ? 44 : 30, overscan: 4, useFlushSync: false });
  const choose = (id?: string) => { onChange(id); close(); };
  const line = "flex w-full items-center gap-2 rounded-control px-2 text-left text-[13px] hover:bg-chrome";
  return <div className="grid gap-1.5 p-1">
    {topics.length > 8 && <input autoFocus className="field-input w-full" aria-label="Find a topic" placeholder="Find a topic…" value={search} onChange={(e) => setSearch(e.target.value)} />}
    <button className={`${line} min-h-7`} aria-pressed={!value} onClick={() => choose(undefined)}><Icon name={value ? "agents" : "check"} size={12} className="shrink-0 text-ink-3" />All sessions</button>
    {rows.length > 0 && <div ref={ref} className="max-h-64 overflow-y-auto"><div style={{ height: virtual.getTotalSize(), position: "relative" }}>{virtual.getVirtualItems().map((v) => {
      const t = rows[v.index];
      return <button key={t.id} style={{ position: "absolute", top: v.start, height: v.size, left: 0 }} className={line} aria-pressed={t.id === value} onClick={() => choose(t.id)}>
        <Icon name={t.id === value ? "check" : "tag"} size={12} className={`shrink-0 ${t.id === value ? "text-teal-ink" : "text-ink-3"}`} />
        <span className="flex min-w-0 flex-1 gap-1"><TopicName topic={t} /></span><span className="shrink-0 text-[12px] text-ink-3 tabular-nums">{t.session_ids.length}</span>
      </button>;
    })}</div></div>}
    {topics.length === 0 && <span className="px-2 text-[12px] text-ink-3">No topics yet: add one from a session’s meta line.</span>}
    <Link to="/topics" onClick={close} className="border-t border-line-2 px-2 pt-1.5 text-[12px] text-teal-ink">All topics</Link>
  </div>;
}

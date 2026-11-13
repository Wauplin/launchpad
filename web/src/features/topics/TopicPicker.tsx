import { useVirtualizer } from "@tanstack/react-virtual";
import { useRef, useState } from "react";
import { Link } from "@tanstack/react-router";
import { Popover } from "@/components/Popover";
import { Sheet } from "@/mobile/Sheet";
import { usePhone } from "@/lib/phone";
import { useScope } from "@/lib/workspace";
import { toast, toastError } from "@/components/Toast";
import { useTopics, useTopicMutations, type Topic } from "./data";

export function TopicPicker({ onPick, exclude = [], label = "Add topic…" }: { onPick: (topic: Topic) => Promise<void> | void; exclude?: string[]; label?: string }) {
  const phone = usePhone();
  const [open, setOpen] = useState(false);
  const content = (close: () => void) => <Picker onPick={onPick} exclude={exclude} close={close} />;
  if (phone) return <><button className="btn btn-sm" onClick={() => setOpen(true)}>{label}</button>{open && <Sheet title="Add topic" onClose={() => setOpen(false)}>{content(() => setOpen(false))}</Sheet>}</>;
  return <Popover viewport width={300} label="Add topic" trigger={({ toggle }) => <button className="btn btn-sm" onClick={toggle}>{label}</button>}>{content}</Popover>;
}
function Picker({ onPick, exclude, close }: { onPick: (topic: Topic) => Promise<void> | void; exclude: string[]; close: () => void }) {
  const query = useTopics();
  const { workspace } = useScope();
  const { create } = useTopicMutations();
  const [search, setSearch] = useState("");
  const [busy, setBusy] = useState(false);
  const topics = query.data ?? [];
  const ref = useRef<HTMLDivElement>(null);
  const phone = usePhone();
  const matches = topics.filter((t) => !exclude.includes(t.id) && t.name.toLocaleLowerCase().includes(search.toLocaleLowerCase()));
  const virtual = useVirtualizer({ count: matches.length, getScrollElement: () => ref.current, estimateSize: () => phone ? 48 : 36, overscan: 4, useFlushSync: false });
  const pick = async (topic?: Topic) => {
    setBusy(true);
    try { await onPick(topic ?? await create(workspace.id, search.trim())); close(); } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  return <div className="grid gap-2 p-3">
    <input autoFocus className="field-input w-full" aria-label="Find or create topic" placeholder="Find or create a topic…" maxLength={80} value={search} onChange={(e) => setSearch(e.target.value)} />
    <div ref={ref} className="max-h-64 overflow-y-auto"><div style={{ height: virtual.getTotalSize(), position: "relative" }}>{virtual.getVirtualItems().map((v) => { const t = matches[v.index]; return <button key={t.id} style={{ position: "absolute", top: v.start, height: v.size, left: 0 }} className="flex w-full items-center justify-between gap-2 rounded-control px-2 text-left text-[13px] hover:bg-chrome" disabled={busy} onClick={() => void pick(t)}><span className="truncate">{t.name}</span><span className="text-ink-3">{t.session_ids.length}</span></button>; })}</div></div>
    {query.isPending && <span className="text-[13px] text-ink-3">Loading topics…</span>}
    {query.error && <span className="text-[13px] text-danger-ink">{query.error.message}</span>}
    {search.trim() && !topics.some((t) => t.name.toLocaleLowerCase() === search.trim().toLocaleLowerCase()) && <button className="btn btn-sm" disabled={busy || !query.data} onClick={() => void pick()}>Create “{search.trim()}”</button>}
    {!search && query.data?.length === 0 && <span className="text-[13px] text-ink-3">Type a name to create your first topic.</span>}
  </div>;
}
export function SessionTopics({ sessionId }: { sessionId: string }) {
  const { data = [] } = useTopics();
  const { add } = useTopicMutations();
  const topics = data.filter((t) => t.session_ids.includes(sessionId));
  const remove = async (t: Topic) => {
    try { await add(t.id, [sessionId], false); toast(`Removed from ${t.name}`, { label: "Undo", run: () => { void add(t.id, [sessionId]).catch(toastError); } }); } catch (e) { toastError(e); }
  };
  return <div className="flex flex-wrap items-center gap-1.5 border-b border-line px-4 py-2" data-tour="session.topics">
    {topics.map((t) => <span key={t.id} className="inline-flex max-w-full items-center rounded-control border border-line text-[12px]"><Link className="truncate px-2 py-1 text-teal-ink hover:underline" to="/topics/$id" params={{ id: t.id }}>{t.name}</Link><button className="px-2 py-1 text-ink-3 hover:text-ink" aria-label={`Remove ${t.name} from this session`} onClick={() => void remove(t)}>×</button></span>)}
    <TopicPicker exclude={topics.map((t) => t.id)} onPick={(t) => add(t.id, [sessionId])} />
  </div>;
}

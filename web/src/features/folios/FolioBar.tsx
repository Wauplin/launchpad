// The reader's bar parts of phase 3: the folio switcher (Folios ▾), the agents of a folio (its topic and the roles of
// `folio_link`), and Ask: the owner's question to the folio's thread with the folio, page, section or quote as
// context. Launch opens the composer with the same reference. Nothing starts without the owner's click.

import { useMemo, useRef, useState } from "react";
import { Link, useNavigate } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { Folio } from "@/gen/Folio";
import type { FolioRef } from "@/gen/FolioRef";
import type { Session } from "@/gen/Session";
import { Icon } from "@/components/Icon";
import { Popover } from "@/components/Popover";
import { Loading, humanError } from "@/components/ui";
import { toast, toastError } from "@/components/Toast";
import { SessionStatus } from "@/features/agents/SessionBits";
import { runAction } from "@/lib/actions";
import { useBots } from "@/lib/bots";
import { ago } from "@/lib/format";
import { folioRef, folioRefLabel, useFolioTargets, useFolios } from "@/lib/folios";
import { usePhone } from "@/lib/phone";
import { useSessions } from "@/lib/queries";
import { useScope } from "@/lib/workspace";
import { Sheet } from "@/mobile/Sheet";
import { useComposer } from "@/stores/composer";

/** What Ask and Launch are about: the page, a section and a quote of it. */
export type FolioPart = { page?: string; section?: string; quote?: string };

/** Launch: the composer, with the folio (or its part) as a reference. */
export function launchWith(f: Folio, rev: number | null, part: FolioPart, text?: string) {
  useComposer.getState().start({ folios: [folioRef(f, { rev: rev ?? f.rev, page: part.page, section: part.section || undefined, quote: part.quote })], text });
}

// ---------------------------------------------------------------- switcher

/** Folios ▾: the workspace's folios, newest change first, with search. */
export function FolioSwitcher({ current }: { current: string }) {
  const navigate = useNavigate();
  return <Popover width={340} viewport label="Folios" trigger={({ open, toggle }) => <button type="button" className="flex items-center gap-1 text-[13px] text-ink-3 hover:text-ink" aria-expanded={open} onClick={toggle} title="Switch to another folio">Folios<Icon name="chev" size={12} /></button>}>
    {(close) => <FolioList current={current} pick={(slug) => { close(); void navigate({ to: "/folios/$slug", params: { slug } }); }} all={() => { close(); void navigate({ to: "/folios" }); }} />}
  </Popover>;
}

function FolioList({ current, pick, all }: { current: string; pick: (slug: string) => void; all: () => void }) {
  const { workspace } = useScope();
  const list = useFolios(workspace.id);
  const [search, setSearch] = useState("");
  const ref = useRef<HTMLDivElement>(null);
  const phone = usePhone();
  const needle = search.trim().toLocaleLowerCase();
  const rows = (list.data ?? []).filter((f) => !needle || `${f.title} ${f.slug} ${f.kind}`.toLocaleLowerCase().includes(needle));
  const virtual = useVirtualizer({ count: rows.length, getScrollElement: () => ref.current, estimateSize: () => (phone ? 48 : 40), overscan: 4, useFlushSync: false });
  return <div className="grid gap-1.5 p-2">
    <input autoFocus className="field-input w-full" aria-label="Find a folio" placeholder="Find a folio…" value={search} onChange={(e) => setSearch(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter" && rows[0]) pick(rows[0].slug); }} />
    {list.isPending && <Loading what="folios" />}
    <div ref={ref} className="max-h-72 overflow-y-auto"><div style={{ height: virtual.getTotalSize(), position: "relative" }}>{virtual.getVirtualItems().map((v) => {
      const f = rows[v.index];
      return <button key={f.id} type="button" style={{ position: "absolute", top: v.start, height: v.size, left: 0 }} className={`grid w-full grid-cols-[minmax(0,1fr)_auto] content-center gap-x-2 rounded-control px-2 text-left hover:bg-chrome ${f.slug === current ? "bg-select" : ""}`} onClick={() => pick(f.slug)}>
        <span className="truncate text-[13px]">{f.title}</span>
        <span className="row-span-2 self-center text-[12px] text-ink-3 tabular-nums">{f.unsent ? `${f.unsent} unsent` : ""}</span>
        <span className="truncate text-[11px] text-ink-3">{f.kind} · rev {f.rev} · {ago(f.updated_at)}</span>
      </button>;
    })}</div></div>
    <button type="button" className="btn btn-sm justify-self-start" onClick={all}>All folios</button>
  </div>;
}

// ---------------------------------------------------------------- agents

const ROLE: Record<string, string> = { created: "created", edited: "edited", replied: "replied", attached: "read it as context" };

/** The agents of a folio: the sessions of `folio_link` with their role and revs, and the folio's bot thread. */
export function FolioAgents({ f, close }: { f: Folio; close: () => void }) {
  const sessions = useSessions().data ?? [];
  const bots = useBots().data ?? [];
  const navigate = useNavigate();
  const byId = useMemo(() => new Map(sessions.map((s) => [s.id, s])), [sessions]);
  const open = (s: Session | undefined, id: string) => {
    close();
    const bot = s?.launch_spec.bot;
    if (bot) void navigate({ to: "/bots/$id", params: { id: bot.id }, search: { thread: bot.thread || undefined } as never });
    else void navigate({ to: "/agents", search: { session: id } as never });
  };
  const thread = f.thread ? bots.find((b) => b.id === f.thread!.bot_id) : null;
  return <div className="grid gap-1 p-2 text-[13px]">
    <span className="px-1 text-[12px] text-ink-3">Agents on this folio{f.topic_id ? <> · <Link to="/topics/$id" params={{ id: f.topic_id }} onClick={close} className="text-teal-ink">its topic</Link></> : null}</span>
    {thread && <button type="button" className="grid grid-cols-[minmax(0,1fr)_auto] gap-x-2 rounded-control px-2 py-1.5 text-left hover:bg-chrome" onClick={() => { close(); void navigate({ to: "/bots/$id", params: { id: thread.id }, search: { thread: f.thread!.thread_id } as never }); }}>
      <b className="truncate">{thread.settings.name}</b><span className="text-[12px] text-ink-3">chat</span>
      <span className="col-span-2 truncate text-[12px] text-ink-3">The folio's chat: Send feedback and Ask go here</span>
    </button>}
    {f.links.map((l) => {
      const s = byId.get(l.session_id);
      return <button key={l.session_id} type="button" className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-2 rounded-control px-2 py-1.5 text-left hover:bg-chrome" onClick={() => open(s, l.session_id)}>
        <span className="truncate">{s?.title ?? l.title ?? "Removed agent"}</span>
        <span className="row-span-2">{s && <SessionStatus s={s} />}</span>
        <span className="truncate text-[12px] text-ink-3">{ROLE[l.role] ?? l.role} · rev {l.first_rev === l.last_rev ? l.first_rev : `${l.first_rev}–${l.last_rev}`} · {ago(l.at)}</span>
      </button>;
    })}
    {!f.links.length && !thread && <span className="px-1 text-[12px] text-ink-3">No agent worked on it yet.</span>}
  </div>;
}

/** N agents ▾ in the reader's bar. */
export function AgentsButton({ f }: { f: Folio }) {
  const phone = usePhone();
  const [open, setOpen] = useState(false);
  const n = f.links.length;
  const label = `${n} agent${n === 1 ? "" : "s"}`;
  if (phone) return <>
    <button type="button" className="btn btn-sm" onClick={() => setOpen(true)}><Icon name="agents" size={12} />{label}</button>
    {open && <Sheet title="Agents on this folio" onClose={() => setOpen(false)}><FolioAgents f={f} close={() => setOpen(false)} /></Sheet>}
  </>;
  return <Popover align="right" width={340} viewport label="Agents on this folio" trigger={({ open: o, toggle }) => <button type="button" className="btn btn-sm" aria-expanded={o} onClick={toggle} title="The agents that created, edited or read this folio"><Icon name="agents" size={12} />{label}<Icon name="chev" size={12} /></button>}>
    {(close) => <FolioAgents f={f} close={close} />}
  </Popover>;
}

// ---------------------------------------------------------------- Ask

type Sent = { session_id: string; bot_id: string | null; thread: string | null };

/** Ask: a question about the folio (or its part) to the folio's thread, a session that wrote it, or a new thread. */
export function AskPanel({ f, rev, part, done }: { f: Folio; rev: number | null; part: FolioPart; done: () => void }) {
  const navigate = useNavigate();
  const targets = useFolioTargets(f.workspace_id, f.slug);
  const bots = (useBots().data ?? []).filter((b) => b.workspace_id === f.workspace_id);
  const [text, setText] = useState("");
  const [choice, setChoice] = useState<string>("0");
  const [busy, setBusy] = useState(false);
  const list = targets.data ?? [];
  const options = [...list.map((t, i) => ({ key: String(i), label: t.label, detail: t.detail, body: { kind: t.kind, bot_id: t.bot_id ?? undefined, thread_id: t.thread_id ?? undefined, session_id: t.session_id ?? undefined } })),
    ...bots.map((b) => ({ key: `new:${b.id}`, label: `New thread of ${b.settings.name}`, detail: "with the agents of this folio as context", body: { kind: "new_thread", bot_id: b.id } }))];
  const picked = options.find((o) => o.key === choice) ?? options[0];
  const ref: FolioRef = folioRef(f, { rev: rev ?? f.rev, page: part.page, section: part.section || undefined, quote: part.quote });
  const send = () => {
    if (!picked || !text.trim()) return;
    setBusy(true);
    void runAction<Sent>("folio.ask", { workspace: f.workspace_id, slug: f.slug }, { text: text.trim(), page: part.page, section: part.section || undefined, quote: part.quote, rev: rev ?? f.rev, ...picked.body })
      .then((r) => {
        toast(`Asked ${picked.label}`, r.bot_id ? { label: "Open the thread", run: () => void navigate({ to: "/bots/$id", params: { id: r.bot_id! }, search: { thread: r.thread ?? undefined } as never }) } : { label: "Open the agent", run: () => void navigate({ to: "/agents", search: { session: r.session_id } as never }) });
        done();
      })
      .catch(toastError).finally(() => setBusy(false));
  };
  return <div className="grid gap-2 p-3 text-[13px]">
    <span className="text-[12px] text-ink-3">About {folioRefLabel(ref)}{part.quote ? <> · “{part.quote.length > 80 ? `${part.quote.slice(0, 79)}…` : part.quote}”</> : null}. The agent gets the folio as a reference and reads what it needs.</span>
    <textarea autoFocus className="field-input min-h-[88px] resize-y" placeholder="Your question" value={text} onChange={(e) => setText(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) send(); }} />
    {targets.isLoading && <Loading what="recipients" />}
    {targets.error && <span className="text-[12px] text-danger">{humanError(targets.error)}</span>}
    {options.length > 0 && <label className="grid gap-1 text-[12px] text-ink-3">Send to
      <select className="field-input" value={picked?.key} onChange={(e) => setChoice(e.target.value)}>
        {options.map((o) => <option key={o.key} value={o.key}>{o.label} · {o.detail}</option>)}
      </select>
    </label>}
    {!targets.isLoading && !options.length && <span className="text-[12px] text-ink-3">No agent to ask: this workspace has no bot and no session wrote this folio. Launch an agent instead.</span>}
    <div className="flex justify-end gap-2">
      <button type="button" className="btn btn-sm" onClick={() => { launchWith(f, rev, part, text.trim() || undefined); done(); }} title="Open the composer with this folio as context">Launch instead</button>
      <button type="button" className="btn btn-sm btn-primary" disabled={busy || !text.trim() || !picked} onClick={send}><Icon name="send" size={12} />Ask</button>
    </div>
  </div>;
}

import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { SessionLog } from "@/lib/sessionStore";
import { EventRow } from "./EventRow";
import { buildRows } from "./rows";

const displayName = (name: string) => name.startsWith("/root/") ? name.split("/").at(-1)! : name;

export type Subagent = { id: string; parent: string | null; name: string; status: string; model: string | null; ts: number };

export function subagents(log: SessionLog | undefined, live: boolean): Subagent[] {
  const agents = new Map<string, Subagent>();
  // Metadata before a tail-first page comes from `earlier`.
  const first = log?.events[0]?.seq ?? Infinity;
  for (const event of [...(log?.earlier?.subagents ?? []).filter((e) => e.seq < first), ...(log?.events ?? [])]) {
    const b = event.body;
    if (b.kind !== "subagent") continue;
    const old = agents.get(b.agent_id);
    agents.set(b.agent_id, {
      id: b.agent_id, parent: b.parent_id ?? old?.parent ?? null,
      name: b.name || old?.name || `Agent ${b.agent_id.slice(-8)}`,
      status: b.status ?? old?.status ?? "unknown", model: b.model ?? old?.model ?? null, ts: event.ts,
    });
  }
  return [...agents.values()].filter((a) => a.status !== "superseded").map((a) => !live && a.status === "running" ? { ...a, status: "unknown" } : a);
}

function childLog(log: SessionLog, id: string): SessionLog {
  const prefix = `${id}\0`;
  const stream = (values: Record<string, string>) => Object.fromEntries(Object.entries(values).filter(([key]) => key.startsWith(prefix)).map(([key, value]) => [key.slice(prefix.length), value]));
  return { ...log, events: log.events.flatMap((e) => e.body.kind === "subagent_event" && e.body.agent_id === id ? [{ ...e, body: e.body.body }] : []), deltas: stream(log.deltas), outputs: stream(log.outputs) };
}

const AgentRow = memo(function AgentRow({ agent, depth, selected, collapsed, hasChildren, select, toggle }: { agent: Subagent; depth: number; selected: boolean; collapsed: boolean; hasChildren: boolean; select: () => void; toggle: () => void }) {
  return <div className={`flex h-12 items-center border-b border-line-2 ${selected ? "bg-select" : "hover:bg-hover"}`} style={{ paddingLeft: 4 + Math.min(depth, 6) * 12 }}>
    <button className="iconbtn shrink-0" disabled={!hasChildren} aria-label={`${collapsed ? "Expand" : "Collapse"} ${agent.name}`} aria-expanded={hasChildren ? !collapsed : undefined} onClick={toggle}>{hasChildren ? <span>{collapsed ? "▸" : "▾"}</span> : <span className="w-3" />}</button>
    <button className="min-w-0 flex-1 px-1 py-1 text-left" aria-pressed={selected} title={`${agent.name} · ${agent.id}`} onClick={select}>
      <span className="block truncate text-xs font-medium">{displayName(agent.name)}</span>
      <span className="flex items-center gap-1 text-[11px] text-ink-3">{agent.status === "running" && <span className="lp-spin" />}<span className={agent.status === "failed" ? "text-gh-closed" : ""}>{agent.status === "unknown" ? "Status unavailable" : agent.status}</span></span>
    </button>
  </div>;
}, (a, b) => a.agent.id === b.agent.id && a.agent.name === b.agent.name && a.agent.status === b.agent.status && a.depth === b.depth && a.selected === b.selected && a.collapsed === b.collapsed && a.hasChildren === b.hasChildren);

export function SubagentsPanel({ log, agents, sessionId, live, back }: { log: SessionLog; agents: Subagent[]; sessionId: string; live: boolean; back: () => void }) {
  const [selected, select] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState<Set<string>>(() => new Set());
  const active = agents.find((a) => a.id === selected) ?? agents.find((a) => a.status === "running") ?? agents[0];
  const tree = useMemo(() => {
    const byId = new Map(agents.map((a) => [a.id, a]));
    const ids = new Set(byId.keys());
    const children = new Map<string | null, Subagent[]>();
    for (const a of agents) {
      const parent = a.parent && a.parent !== a.id && ids.has(a.parent) ? a.parent : null;
      const siblings = children.get(parent) ?? [];
      siblings.push(a);
      children.set(parent, siblings);
    }
    const out: { agent: Subagent; depth: number; hasChildren: boolean }[] = [];
    const seen = new Set<string>();
    const walk = (parent: string | null, depth: number) => {
      for (const agent of children.get(parent) ?? []) {
        if (seen.has(agent.id)) continue;
        seen.add(agent.id);
        out.push({ agent, depth, hasChildren: !!children.get(agent.id)?.length });
        if (!collapsed.has(agent.id)) walk(agent.id, depth + 1);
      }
    };
    walk(null, 0);
    // Defensive fallback for malformed/cyclic ancestry, without resurrecting collapsed children.
    for (const agent of agents) {
      if (seen.has(agent.id)) continue;
      const ancestry = new Set<string>();
      let parent = agent.parent;
      let hidden = false;
      while (parent && !ancestry.has(parent)) {
        ancestry.add(parent);
        if (collapsed.has(parent)) { hidden = true; break; }
        parent = byId.get(parent)?.parent ?? null;
      }
      if (!hidden) { seen.add(agent.id); out.push({ agent, depth: 0, hasChildren: !!children.get(agent.id)?.length }); if (!collapsed.has(agent.id)) walk(agent.id, 1); }
    }
    return out;
  }, [agents, collapsed]);
  useEffect(() => { if (selected === null && active) select(active.id); }, [selected, active?.id]);
  const rows = useMemo(() => active ? buildRows(childLog(log, active.id)) : [], [log, active?.id]);
  const listRef = useRef<HTMLDivElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const bottom = useRef(true);
  const list = useVirtualizer({ count: tree.length, getScrollElement: () => listRef.current, estimateSize: () => 48, overscan: 4, useFlushSync: false });
  const trace = useVirtualizer({ count: rows.length, getScrollElement: () => scrollRef.current, estimateSize: () => 48, overscan: 6, useFlushSync: false });
  const total = trace.getTotalSize();
  useEffect(() => { bottom.current = true; }, [active?.id]);
  useLayoutEffect(() => { if (bottom.current && rows.length) trace.scrollToIndex(rows.length - 1, { align: "end" }); }, [rows.length, total, active?.id, trace]);
  const parent = agents.find((a) => a.id === active?.parent);
  return <div className="@container flex min-h-0 flex-1 flex-col">
    <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-line-2 px-3 py-2 text-xs text-ink-3">
      <button className="btn btn-sm" onClick={back}>← Parent trace</button>
      <span className="whitespace-nowrap">{agents.filter((a) => a.status === "running").length} running · {agents.length} total</span>
      <span className="ml-auto whitespace-nowrap">Read-only</span>
    </div>
    <div className="flex min-h-0 flex-1 flex-col @min-[600px]:flex-row">
      <div ref={listRef} style={{ "--agent-list-height": `${Math.min(3, tree.length) * 48}px` } as React.CSSProperties} className="h-[var(--agent-list-height)] shrink-0 overflow-y-auto border-b border-line-2 @min-[600px]:h-auto @min-[600px]:w-56 @min-[600px]:border-r @min-[600px]:border-b-0" aria-label="Subagents">
        <div style={{ height: list.getTotalSize(), position: "relative" }}>
          {list.getVirtualItems().map((it) => { const row = tree[it.index]; return <div key={row.agent.id} style={{ position: "absolute", top: 0, left: 0, right: 0, transform: `translateY(${it.start}px)` }}><AgentRow {...row} selected={active?.id === row.agent.id} collapsed={collapsed.has(row.agent.id)} select={() => select(row.agent.id)} toggle={() => setCollapsed((old) => { const next = new Set(old); if (!next.delete(row.agent.id)) next.add(row.agent.id); return next; })} /></div>; })}
        </div>
      </div>
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        {active && <div className="shrink-0 border-b border-line-2 px-3 py-2">
          {parent && <button className="mb-1 block max-w-full truncate text-[11px] text-teal-ink" onClick={() => select(parent.id)}>← {displayName(parent.name)}</button>}
          <div className="truncate text-xs font-semibold" title={active.name}>{displayName(active.name)}</div>
          <div className="flex flex-wrap gap-x-2 text-[11px] text-ink-3"><span>{active.status === "unknown" ? "Status unavailable" : active.status}</span>{active.model && <span>{active.model}</span>}<span title={active.id}>{active.id.slice(-8)}</span></div>
        </div>}
        <div key={active?.id} ref={scrollRef} className="min-h-0 flex-1 overflow-y-auto px-3" onScroll={(e) => { const el = e.currentTarget; bottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 60; }}>
          {!rows.length && <div className="p-6 text-center text-xs text-ink-3">{active?.status === "running" ? "Waiting for trace events…" : "The harness supplied no trace events for this agent."}</div>}
          <div style={{ height: total, position: "relative" }}>
            {trace.getVirtualItems().map((it) => <div key={`${active?.id}-${rows[it.index].key}`} data-index={it.index} ref={trace.measureElement} className="flex flex-col py-[5px]" style={{ position: "absolute", top: 0, left: 0, right: 0, transform: `translateY(${it.start}px)` }}><EventRow row={rows[it.index]} sessionId={sessionId} live={false} /></div>)}
          </div>
          {active?.status === "running" && live && <div className="flex items-center gap-2 py-2 text-xs text-work"><span className="lp-spin" />Working…</div>}
        </div>
      </div>
    </div>
  </div>;
}

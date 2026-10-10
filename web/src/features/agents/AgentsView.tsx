// Agents (sketch S3): sessions grouped by state, showing one session at a time.
// Clicking a session opens its conversation. The window's workspace narrows every group.

import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { useNavigate, useSearch } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useEffect, useRef, useState } from "react";
import type { ScheduledLaunch } from "@/gen/ScheduledLaunch";
import { conditionsSummary } from "@/features/launch/DelaySelector";
import { ScheduledPane, useSchedules } from "./ScheduledPane";
import { CronPane, cronSummary } from "./CronPane";
import type { CronAgent } from "@/gen/CronAgent";
import type { Session } from "@/gen/Session";
import { HarnessMark } from "@/components/HarnessMark";
import { isRunning, isDone, isQuotaPaused, isStopped } from "@/lib/sessionStatus";
import { Icon } from "@/components/Icon";
import { LaunchButton } from "@/features/launch/LaunchButton";
import { projectName, useCrons, useProjects, useSessions } from "@/lib/queries";
import { useScope } from "@/lib/workspace";
import { sendPresence } from "@/lib/ws";
import { useLayout, type AgentGroup } from "@/stores/layout";
import { SessionPane } from "./SessionPane";
import { StatusBadge } from "./SessionBits";

const SessionRow = memo(function SessionRow({ s, project, selected, pinned, onClick }: { s: Session; project: string; selected: boolean; pinned: boolean; onClick: () => void }) {
  const dim = s.status === "closed";
  const quotaDetail = s.quota_pause?.error ?? s.quota_pause?.blockers.map((b) => `${b.window} reset ${new Date(b.resets_at * 1000).toLocaleString([], { weekday: "short", hour: "2-digit", minute: "2-digit" })}`).join(" · ");
  return (
    <button
      onClick={onClick}
      className={`grid h-[68px] w-full grid-cols-[18px_1fr] gap-x-[9px] gap-y-[3px] border-b border-line-2 px-3 py-2 text-left hover:bg-[#f5f8fb] ${selected ? "bg-select!" : ""}`}
    >
      <span className="row-span-2 mt-px">
        <HarnessMark harness={s.harness} size={18} />
      </span>
      <span className={`truncate text-[12.5px] leading-[1.35] ${dim ? "font-medium text-ink-2" : "font-semibold"}`} title={s.title}>
        {pinned ? "★ " : ""}{s.title}
      </span>
      <span className="flex items-center gap-x-2 overflow-hidden whitespace-nowrap text-[11.5px] text-ink-3">
        {s.quota_pause ? <span className="truncate text-attn" title={`${project} · ${quotaDetail}`}>{quotaDetail || "Waiting for a quota update"}</span> : <>
          <span className="max-w-[110px] truncate">{project}</span>
          <StatusBadge s={s} />
          {s.usage.cost_usd != null && <span>${s.usage.cost_usd.toFixed(2)}</span>}
        </>}
      </span>
    </button>
  );
});

const CronRow = memo(function CronRow({ cron, selected, onClick }: { cron: CronAgent; selected: boolean; onClick: () => void }) {
  const skipped = !!cron.last_outcome && !cron.last_outcome.startsWith("Started");
  return <button onClick={onClick} className={`grid h-[68px] w-full gap-1 border-b border-line px-3 py-2 text-left hover:bg-chrome ${selected ? "bg-select" : ""}`}>
    <span className="truncate text-xs font-semibold" title={cron.title}>{cron.title}</span>
    <span className="truncate text-[11px] text-ink-3" title={cron.last_outcome ?? undefined}>{skipped ? "Last run skipped · " : ""}{cronSummary(cron)}</span>
  </button>;
});

function Group({ title, count, highlight, collapsed, onToggle }: { title: AgentGroup; count: number; highlight?: boolean; collapsed: boolean; onToggle: () => void }) {
  return (
    <button
      type="button"
      aria-expanded={!collapsed}
      aria-label={`${collapsed ? "Expand" : "Collapse"} ${title}`}
      onClick={onToggle}
      className="flex h-[34px] w-full items-center gap-2 px-3 text-left seclabel hover:bg-sunk"
    >
      <Icon name="chev" size={12} className={collapsed ? "-rotate-90" : ""} />
      {title} <b className={highlight && count ? "text-attn" : "font-normal text-ink-3"}>{count}</b>
    </button>
  );
}

export function AgentsView() {
  const scope = useScope();
  const allSessions = useSessions().data ?? [];
  const sessions = allSessions.filter(scope.hasSession);
  const [panelLoad, setPanelLoad] = useState(0);
  const [filter, setFilter] = useState("");
  const [debounced, setDebounced] = useState("");
  const [pinsOnly, setPinsOnly] = useState(false);
  const pinned = useLayout((s) => s.pinned);
  useEffect(() => { const t = setTimeout(() => setDebounced(filter.trim()), 250); return () => clearTimeout(t); }, [filter]);
  const matches = useQuery({ queryKey: ["session-search", debounced], queryFn: () => api.get<string[]>(`/api/sessions/search?q=${encodeURIComponent(debounced)}`), enabled: !!debounced });
  const schedules = (useSchedules().data ?? []).filter((s) => scope.hasSpec(s.spec));
  const [scheduledId, setScheduledIdState] = useState<string | null>(null);
  const scheduled = schedules.find((s) => s.id === scheduledId);
  const scheduledTarget = useRef<{ id: string; session: string } | null>(null);
  const crons = (useCrons().data ?? []).filter((c) => scope.hasSpec(c.spec));
  const [cronId, setCronIdState] = useState<string | null>(null);
  const cron = crons.find((c) => c.id === cronId);
  // A cron and a scheduled launch share the detail panel, so selecting one clears the other.
  const setCronId = (id: string | null) => { setCronIdState(id); if (id) setScheduledIdState(null); };
  const setScheduledId = (id: string | null) => { setScheduledIdState(id); if (id) setCronIdState(null); };
  const clearSide = () => { setScheduledIdState(null); setCronIdState(null); };
  useEffect(() => {
    if (scheduled) scheduledTarget.current = { id: scheduled.id, session: scheduled.resume_session_id ?? scheduled.id };
    if (scheduledId && !scheduled) {
      const id = scheduledTarget.current?.id === scheduledId ? scheduledTarget.current.session : scheduledId;
      if (allSessions.some((s) => s.id === id)) useLayout.getState().show(id);
      setScheduledIdState(null);
    }
  }, [scheduledId, scheduled, allSessions]);
  useEffect(() => { if (cronId && !crons.some((c) => c.id === cronId)) setCronIdState(null); }, [cronId, crons]);
  const projects = useProjects().data;
  const { selectedSession, show, clearSession, collapsedGroups, toggleGroup } = useLayout();
  const search = useSearch({ strict: false }) as { session?: string };
  const navigate = useNavigate();
  const [archivedLimit, setArchivedLimit] = useState(30);
  const listRef = useRef<HTMLDivElement>(null);

  // From a desktop notification: /agents?session=<id>.
  useEffect(() => {
    if (search.session) {
      clearSide();
      show(search.session);
      setPanelLoad((n) => n + 1);
      void navigate({ to: "/agents", search: {}, replace: true });
    }
  }, [search.session, show, navigate]);

  // Tell the daemon which sessions are on screen, so it does not notify about them.
  const visibleSession = !cron && !scheduled && sessions.some((s) => s.id === selectedSession) ? selectedSession : null;
  useEffect(() => {
    const report = () => sendPresence(document.hasFocus() && document.visibilityState === "visible", visibleSession ? [visibleSession] : []);
    report();
    window.addEventListener("focus", report);
    window.addEventListener("blur", report);
    document.addEventListener("visibilitychange", report);
    return () => {
      window.removeEventListener("focus", report);
      window.removeEventListener("blur", report);
      document.removeEventListener("visibilitychange", report);
      sendPresence(false, []);
    };
  }, [visibleSession]);

  const needle = filter.trim().toLocaleLowerCase();
  const filtered = sessions.filter((s) => (!pinsOnly || pinned.includes(s.id)) && (!needle || [s.title, s.cwd, s.prompt, projectName(projects, s.project_id, s.cwd)].some((t) => t.toLocaleLowerCase().includes(needle)) || (debounced === filter.trim() && matches.data?.includes(s.id))));
  const order = (a: Session, b: Session) => Number(pinned.includes(b.id)) - Number(pinned.includes(a.id)) || b.started_at - a.started_at;
  const filteredCrons = pinsOnly ? [] : crons.filter((c) => !needle || `${c.title} ${c.spec.text} ${c.schedule}`.toLocaleLowerCase().includes(needle));
  const filteredSchedules = pinsOnly ? [] : schedules.filter((s) => !needle || `${s.title} ${s.spec.text}`.toLocaleLowerCase().includes(needle));
  const done = filtered.filter((s) => isDone(s)).sort((a, b) => Number(pinned.includes(b.id)) - Number(pinned.includes(a.id)) || a.status_since - b.status_since);
  const running = filtered.filter((s) => isRunning(s)).sort(order);
  const quotaPaused = filtered.filter(isQuotaPaused).sort(order);
  const stopped = filtered.filter(isStopped).sort(order);
  const interrupted = filtered.filter((s) => s.status === "interrupted").sort(order);
  const failed = filtered.filter((s) => s.status === "error").sort(order);
  const inTerminal = filtered.filter((s) => s.status === "in_terminal").sort(order);
  const archived = filtered.filter((s) => s.status === "closed").sort(order);
  type Item = { kind: "group"; title: AgentGroup; count: number; highlight?: boolean } | { kind: "session"; session: Session } | { kind: "schedule"; launch: ScheduledLaunch } | { kind: "cron"; cron: CronAgent } | { kind: "more" };
  // Groups with no items are not shown at all.
  const group = (title: AgentGroup, rows: Item[], highlight?: boolean): Item[] => rows.length ? [{ kind: "group", title, count: rows.length, highlight }, ...(collapsedGroups[title] ? [] : rows)] : [];
  const sessionRows = (list: Session[]) => list.map((session) => ({ kind: "session" as const, session }));
  const items: Item[] = [
    ...group("Cron", filteredCrons.map((cron) => ({ kind: "cron" as const, cron }))),
    ...group("Scheduled", filteredSchedules.map((launch) => ({ kind: "schedule" as const, launch }))),
    ...group("Running", sessionRows(running)),
    ...group("Quota paused", sessionRows(quotaPaused)),
    ...group("Stopped", sessionRows(stopped)),
    ...group("Interrupted", sessionRows(interrupted)),
    ...group("Failed", sessionRows(failed)),
    ...group("In terminal", sessionRows(inTerminal)),
    ...group("Done", sessionRows(done), true),
    ...(archived.length ? [{ kind: "group" as const, title: "Archived" as const, count: archived.length }] : []),
    ...(collapsedGroups.Archived ? [] : sessionRows(archived.slice(0, archivedLimit))),
    ...(!collapsedGroups.Archived && archived.length > archivedLimit ? [{ kind: "more" as const }] : []),
  ];
  const virtual = useVirtualizer({ count: items.length, getScrollElement: () => listRef.current, estimateSize: (i) => items[i].kind === "session" || items[i].kind === "schedule" || items[i].kind === "cron" ? 68 : 34, getItemKey: (i) => items[i].kind === "session" ? items[i].session.id : items[i].kind === "schedule" ? items[i].launch.id : items[i].kind === "cron" ? `cron-${items[i].cron.id}` : items[i].kind === "group" ? `group-${items[i].title}` : "more-archived", overscan: 6, useFlushSync: false });
  const name = (s: Session) => projectName(projects, s.project_id, s.cwd);
  const row = (s: Session) => <SessionRow key={s.id} s={s} project={name(s)} pinned={pinned.includes(s.id)} selected={s.id === visibleSession} onClick={() => { clearSide(); show(s.id); setPanelLoad((n) => n + 1); }} />;

  return (
    <div className="flex h-full min-h-0">
      <div className="flex w-[280px] flex-none flex-col border-r border-line bg-paper">
        <div className="grid gap-2 border-b border-line p-2">
          <input aria-label="Search sessions" className="field-input w-full" placeholder="Search sessions and messages…" value={filter} onChange={(e) => { setFilter(e.target.value); setArchivedLimit(30); }} />
          <div className="flex gap-2"><button className={`btn btn-sm ${pinsOnly ? "bg-teal-soft!" : ""}`} aria-pressed={pinsOnly} onClick={() => setPinsOnly(!pinsOnly)}>★ Pinned</button></div>
          {matches.isFetching && <span className="text-xs text-ink-3">Searching messages…</span>}
          {matches.error && <span className="text-xs text-danger">{matches.error.message}</span>}
        </div>
        <div ref={listRef} className="min-h-0 flex-1 overflow-y-auto">
        {sessions.length === 0 && schedules.length === 0 && crons.length === 0 && (
          <div className="grid gap-3 p-4 text-[12.5px] text-ink-2">
            {scope.workspace ? `No agent in ${scope.workspace.name} yet.` : "No session yet."}
            <span>
              <LaunchButton small />
            </span>
          </div>
        )}
        <div style={{ height: virtual.getTotalSize(), position: "relative", flexShrink: 0 }}>
          {virtual.getVirtualItems().map((v) => {
            const item = items[v.index];
            return <div key={v.key} style={{ position: "absolute", top: 0, left: 0, width: "100%", height: v.size, transform: `translateY(${v.start}px)` }}>
              {item.kind === "session" ? row(item.session) : item.kind === "cron" ? <CronRow cron={item.cron} selected={cronId === item.cron.id} onClick={() => setCronId(item.cron.id)} /> : item.kind === "schedule" ? <button onClick={() => setScheduledId(item.launch.id)} className={`grid h-[68px] w-full gap-1 border-b border-line px-3 py-2 text-left hover:bg-chrome ${scheduledId === item.launch.id ? "bg-select" : ""}`}><span className="truncate text-xs font-semibold">{item.launch.title}</span><span className="truncate text-[11px] text-ink-3" title={conditionsSummary(item.launch, sessions, schedules)}>{item.launch.status === "failed" ? "Needs attention · " : item.launch.status === "starting" ? "Starting · " : ""}{conditionsSummary(item.launch, sessions, schedules)}</span></button> : item.kind === "group" ? <Group title={item.title} count={item.count} highlight={item.highlight} collapsed={!!collapsedGroups[item.title]} onToggle={() => toggleGroup(item.title)} /> :
                <button className="px-3 py-2 text-left text-xs text-teal-ink hover:underline" onClick={() => setArchivedLimit((n) => n + 50)}>Show more</button>}
            </div>;
          })}
        </div>
        </div>
      </div>
      <div className="min-h-0 min-w-0 flex-1 bg-paper">
        {cron ? (
          <CronPane key={cron.id} cron={cron} onClose={() => setCronId(null)} onOpen={(s) => { clearSide(); show(s.id); setPanelLoad((n) => n + 1); }} />
        ) : scheduled ? (
          <ScheduledPane key={scheduled.id} launch={scheduled} onClose={clearSide} onStart={(s) => { clearSide(); show(s.id); setPanelLoad((n) => n + 1); }} />
        ) : visibleSession ? (
          <SessionPane key={`${scope.workspace?.id ?? "all"}:${visibleSession}:${panelLoad}`} sessionId={visibleSession} onClear={() => clearSession(visibleSession)} />
        ) : (
          <div className="grid h-full place-items-center text-[12.5px] text-ink-3">Pick a session on the left, or launch a new one.</div>
        )}
      </div>
    </div>
  );
}

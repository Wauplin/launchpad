// Agents (sketch S3): sessions grouped by state, Needs you first, showing one session at a time.
// Clicking a session opens its conversation. The window's workspace narrows every group.

import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { useNavigate, useSearch } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useEffect, useRef, useState, type ReactNode } from "react";
import type { ScheduledLaunch } from "@/gen/ScheduledLaunch";
import { conditionsSummary } from "@/features/launch/DelaySelector";
import { ScheduledPane, useSchedules } from "./ScheduledPane";
import { CronPane, cronSummary } from "./CronPane";
import type { CronAgent } from "@/gen/CronAgent";
import type { Session } from "@/gen/Session";
import { HarnessMark } from "@/components/HarnessMark";
import { isRunning, isDone, isQuotaPaused, isStopped, isUnseen, needsYou } from "@/lib/sessionStatus";
import { Icon } from "@/components/Icon";
import { LaunchButton } from "@/features/launch/LaunchButton";
import { projectName, useCrons, useProjects, useSessions } from "@/lib/queries";
import { useScope } from "@/lib/workspace";
import { sendPresence } from "@/lib/ws";
import { isCollapsed, useLayout, type AgentGroup } from "@/stores/layout";
import { SessionPane } from "./SessionPane";
import { StatusBadge } from "./SessionBits";
import { useMarkSeen, useWindowFocused } from "./useSeen";
import { formatAge, formatReset, resetLabel } from "@/lib/format";
import { toastError } from "@/components/Toast";
import { useHarnesses } from "@/components/HarnessMark";
import { useStatus } from "@/app/StatusBar";

/** Statuses whose badge tells more than the group heading. */
const informative = (s: Session) => s.open_question || s.status === "error" || s.status === "interrupted" || s.status === "quota_paused";

const SessionRow = memo(function SessionRow({ s, project, selected, pinned, onClick, onMarkUnread }: { s: Session; project: string; selected: boolean; pinned: boolean; onClick: () => void; onMarkUnread: () => void }) {
  const [acting, setActing] = useState(false);
  const dim = s.status === "closed";
  const stop = isRunning(s) || isQuotaPaused(s);
  const canMarkUnread = !isUnseen(s) && (isDone(s) || s.status === "error" || s.status === "interrupted");
  const action = stop ? "Stop" : "Archive";
  const act = async (unread = false) => {
    setActing(true);
    try {
      if (unread) onMarkUnread();
      await api.post(`/api/sessions/${s.id}/${unread ? "unread" : stop ? "interrupt" : "close"}`);
    }
    catch (e) { toastError(e); }
    finally { setActing(false); }
  };
  const needs = needsYou(s);
  const quotaDetail = s.quota_pause?.error ?? s.quota_pause?.blockers.map((b) => `${b.window} reset ${b.resets_at === null ? "unknown" : new Date(b.resets_at * 1000).toLocaleString([], { weekday: "short", hour: "2-digit", minute: "2-digit" })}`).join(" · ");
  const line = s.quota_pause ? quotaDetail || "Waiting for a quota update" : s.last_message ?? (s.usage.cost_usd != null ? `$${s.usage.cost_usd.toFixed(2)}` : "");
  return (
    <div className="group relative">
      <button
        type="button"
        onClick={onClick}
        className={`grid h-[68px] w-full grid-cols-[18px_1fr_auto] content-start gap-x-[9px] gap-y-[3px] border-b border-line-2 px-3 py-2 text-left hover:bg-[#f5f8fb] ${selected ? "bg-select!" : ""}`}
      >
        <span className="row-span-2 mt-px">
          <HarnessMark harness={s.harness} size={18} />
        </span>
        <span className={`truncate text-[12.5px] leading-[1.35] ${dim ? "font-medium text-ink-2" : needs ? "font-semibold" : "font-medium"}`} title={s.title}>
          {pinned ? "★ " : ""}{s.title}
        </span>
        <span className="flex items-center gap-1 text-[11.5px] whitespace-nowrap text-ink-3">
          {informative(s) ? <StatusBadge s={s} /> : <>{isRunning(s) && <span className="lp-spin" title="Running" />}{formatAge(dim ? s.ended_at ?? s.status_since : s.status_since)}</>}
        </span>
        <span className={`col-span-2 line-clamp-2 text-[11.5px] leading-[1.3] text-ink-3 ${dim ? "" : canMarkUnread ? "pr-14" : "pr-7"}`} title={line ? `${project} · ${line}` : project}>
          <span className="text-ink-2">{project}</span>{line && <> · <span className={s.quota_pause ? "text-attn" : ""}>{line}</span></>}
        </span>
      </button>
      {canMarkUnread && <button
        type="button"
        title="Mark unread for later"
        aria-label={`Mark ${s.title} unread`}
        disabled={acting}
        onClick={() => void act(true)}
        className="absolute bottom-2 right-9 flex h-6 w-6 items-center justify-center rounded text-ink-3 opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 hover:bg-sunk hover:text-ink disabled:cursor-wait [@media(hover:none)]:opacity-100"
      >
        <Icon name="mail" size={14} />
      </button>}
      {!dim && <button
        type="button"
        title={`${action} session`}
        aria-label={`${action} ${s.title}`}
        disabled={acting}
        onClick={() => void act()}
        className={`absolute bottom-2 right-2 flex h-6 w-6 items-center justify-center rounded text-ink-3 opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 hover:bg-sunk hover:text-ink disabled:cursor-wait [@media(hover:none)]:opacity-100 ${acting ? "opacity-100" : ""}`}
      >
        {acting ? <span className="lp-spin" /> : <Icon name={stop ? "stop" : "archive"} size={14} />}
      </button>}
    </div>
  );
});

const CronRow = memo(function CronRow({ cron, selected, onClick }: { cron: CronAgent; selected: boolean; onClick: () => void }) {
  const skipped = !!cron.last_outcome && !cron.last_outcome.startsWith("Started");
  return <button onClick={onClick} className={`grid h-[68px] w-full gap-1 border-b border-line px-3 py-2 text-left hover:bg-chrome ${selected ? "bg-select" : ""}`}>
    <span className="truncate text-xs font-semibold" title={cron.title}>{cron.title}</span>
    <span className="truncate text-[11px] text-ink-3" title={cron.last_outcome ?? undefined}>{skipped ? "Last run skipped · " : ""}{cronSummary(cron)}</span>
  </button>;
});

function Group({ title, count, highlight, detail, collapsed, onToggle }: { title: AgentGroup; count: number; highlight?: boolean; detail?: string; collapsed: boolean; onToggle: () => void }) {
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
      {detail && <span className="ml-auto font-normal normal-case tracking-normal text-ink-3">{detail}</span>}
    </button>
  );
}

const DAY = 86400_000;
const earliest = (times: (number | null)[]) => times.filter((t): t is number => t != null).sort((a, b) => a - b)[0] ?? null;

function HomeSection({ title, children }: { title: string; children: ReactNode }) {
  return <section className="grid gap-1.5"><div className="seclabel">{title}</div>{children}</section>;
}

function HomeRow({ s, project, onOpen }: { s: Session; project: string; onOpen: () => void }) {
  return <button onClick={onOpen} className="flex h-[30px] w-full items-center gap-2 rounded-md px-2 text-left hover:bg-sunk">
    <HarnessMark harness={s.harness} size={14} />
    <span className="min-w-0 truncate font-medium">{s.title}</span>
    <span className="max-w-[140px] flex-none truncate text-ink-3">{project}</span>
    <span className="ml-auto flex flex-none items-center gap-1 text-ink-3">{informative(s) ? <StatusBadge s={s} /> : <>{isRunning(s) && <span className="lp-spin" />}{formatAge(s.status_since)}</>}</span>
  </button>;
}

/** The pane when no session is selected: what waits, what runs, what comes next, and the quotas. */
function AgentsHome({ sessions, crons, schedules, name, onOpen, onCron, onSchedule }: { sessions: Session[]; crons: CronAgent[]; schedules: ScheduledLaunch[]; name: (s: Session) => string; onOpen: (id: string) => void; onCron: (id: string) => void; onSchedule: (id: string) => void }) {
  const status = useStatus().data;
  const harnesses = useHarnesses().filter((h) => h.capabilities.quota);
  const needs = sessions.filter(needsYou).sort((a, b) => b.status_since - a.status_since);
  const running = sessions.filter(isRunning).sort((a, b) => b.status_since - a.status_since);
  const quotaPaused = sessions.filter(isQuotaPaused).sort((a, b) => b.status_since - a.status_since);
  const nextCron = crons.filter((c) => !c.paused && c.next_run_at != null).sort((a, b) => a.next_run_at! - b.next_run_at!)[0];
  const upcoming = [
    ...schedules.filter((l) => l.status === "queued").map((l) => ({
      id: `scheduled-${l.id}`, title: l.title, at: l.ready_at,
      detail: conditionsSummary(l, sessions, schedules) || "Waiting for conditions",
      open: () => onSchedule(l.id),
    })),
    ...(nextCron ? [{
      id: `cron-${nextCron.id}`, title: nextCron.title, at: nextCron.next_run_at,
      detail: `Cron · ${formatReset(nextCron.next_run_at! / 1000)}`,
      open: () => onCron(nextCron.id),
    }] : []),
  ].sort((a, b) => (a.at ?? Infinity) - (b.at ?? Infinity));
  const pct = (q: { used_percent: number } | null) => (q ? `${Math.round(q.used_percent)}%` : "unknown");
  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto grid max-w-[560px] gap-5 px-6 py-8 text-[12.5px]">
        <div className="flex items-center gap-3">
          <h2 className="text-[15px] font-semibold">{needs.length ? `${needs.length} ${needs.length === 1 ? "session needs" : "sessions need"} you` : "Nothing waits for you"}</h2>
          <span className="ml-auto"><LaunchButton small /></span>
        </div>
        {needs.length > 0 && <HomeSection title="Needs you">{needs.slice(0, 5).map((s) => <HomeRow key={s.id} s={s} project={name(s)} onOpen={() => onOpen(s.id)} />)}{needs.length > 5 && <span className="px-2 text-ink-3">{needs.length - 5} more in the list on the left</span>}</HomeSection>}
        <HomeSection title={`Running · ${running.length}`}>
          {running.length ? running.slice(0, 8).map((s) => <HomeRow key={s.id} s={s} project={name(s)} onOpen={() => onOpen(s.id)} />) : <span className="px-2 text-ink-3">No agent is running.</span>}
          {running.length > 8 && <span className="px-2 text-ink-3">{running.length - 8} more in the list on the left</span>}
        </HomeSection>
        <HomeSection title={`Quota paused · ${quotaPaused.length}`}>
          {quotaPaused.length ? quotaPaused.slice(0, 8).map((s) => <HomeRow key={s.id} s={s} project={name(s)} onOpen={() => onOpen(s.id)} />) : <span className="px-2 text-ink-3">No agent is quota paused.</span>}
          {quotaPaused.length > 8 && <span className="px-2 text-ink-3">{quotaPaused.length - 8} more in the list on the left</span>}
        </HomeSection>
        <HomeSection title={`Scheduled launches · ${upcoming.length}`}>
          {upcoming.length ? upcoming.slice(0, 5).map((launch) => <button key={launch.id} onClick={launch.open} className="grid h-[46px] w-full grid-cols-[14px_minmax(0,1fr)] items-center gap-x-2 rounded-md px-2 py-1 text-left hover:bg-sunk">
            <Icon name="clock" size={14} className="row-span-2" />
            <span className="truncate font-medium" title={launch.title}>{launch.title}</span>
            <span className="truncate text-ink-3" title={launch.detail}>{launch.detail}</span>
          </button>) : <span className="px-2 text-ink-3">Nothing scheduled.</span>}
          {upcoming.length > 5 && <span className="px-2 text-ink-3">{upcoming.length - 5} more in the list on the left</span>}
        </HomeSection>
        {harnesses.length > 0 && <HomeSection title="Quota">
          {harnesses.map((h) => {
            const q = status?.harnesses[h.id];
            return <div key={h.id} className="flex items-center gap-2 px-2"><HarnessMark harness={h.id} size={14} /><span className="font-medium">{h.name}</span><span className="ml-auto text-ink-3">5 h {pct(q?.five_hour ?? null)}{q?.five_hour && ` · ${resetLabel(q.five_hour.resets_at)}`} · week {pct(q?.weekly ?? null)}{q?.weekly && ` · ${resetLabel(q.weekly.resets_at)}`}</span></div>;
          })}
        </HomeSection>}
      </div>
    </div>
  );
}

export function AgentsView() {
  const scope = useScope();
  const sessionsQuery = useSessions();
  const allSessions = sessionsQuery.data ?? [];
  const sessions = allSessions.filter(scope.hasSession);
  const [panelLoad, setPanelLoad] = useState(0);
  const [filter, setFilter] = useState("");
  const [debounced, setDebounced] = useState("");
  const [pinsOnly, setPinsOnly] = useState(false);
  const [archiving, setArchiving] = useState(false);
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
  const open = (id: string) => { clearSide(); show(id); setPanelLoad((n) => n + 1); };

  // From a desktop notification: /agents?session=<id>.
  useEffect(() => {
    if (search.session) {
      clearSide();
      show(search.session);
      setPanelLoad((n) => n + 1);
      void navigate({ to: "/agents", search: {}, replace: true });
    }
  }, [search.session, show, navigate]);

  // Tell the daemon which sessions are on screen, so it does not notify about them, and mark the shown session seen.
  const focused = useWindowFocused();
  const visibleSession = !cron && !scheduled && sessions.some((s) => s.id === selectedSession) ? selectedSession : null;
  const visible = sessions.find((s) => s.id === visibleSession);
  useEffect(() => { sendPresence(focused, visibleSession ? [visibleSession] : []); }, [focused, visibleSession]);
  useEffect(() => () => sendPresence(false, []), []);
  useMarkSeen(visible, focused);

  const needle = filter.trim().toLocaleLowerCase();
  const filtered = sessions.filter((s) => (!pinsOnly || pinned.includes(s.id)) && (!needle || [s.title, s.cwd, s.prompt, s.last_message ?? "", projectName(projects, s.project_id, s.cwd)].some((t) => t.toLocaleLowerCase().includes(needle)) || (debounced === filter.trim() && matches.data?.includes(s.id))));
  const pinFirst = (a: Session, b: Session) => Number(pinned.includes(b.id)) - Number(pinned.includes(a.id));
  const order = (a: Session, b: Session) => pinFirst(a, b) || b.started_at - a.started_at;
  const newest = (a: Session, b: Session) => pinFirst(a, b) || b.status_since - a.status_since;
  const filteredCrons = pinsOnly ? [] : crons.filter((c) => !needle || `${c.title} ${c.spec.text} ${c.schedule}`.toLocaleLowerCase().includes(needle));
  const filteredSchedules = pinsOnly ? [] : schedules.filter((s) => !needle || `${s.title} ${s.spec.text}`.toLocaleLowerCase().includes(needle));
  // Needs you: open questions, then unseen Done (newest first), then unseen Failed and Interrupted.
  const needRank = (s: Session) => (s.open_question ? 0 : isDone(s) ? 1 : 2);
  const needs = filtered.filter(needsYou).sort((a, b) => needRank(a) - needRank(b) || b.status_since - a.status_since);
  const done = filtered.filter((s) => isDone(s) && !needsYou(s)).sort(newest);
  const running = filtered.filter((s) => isRunning(s)).sort(order);
  const quotaPaused = filtered.filter(isQuotaPaused).sort(order);
  const stopped = filtered.filter(isStopped).sort(order);
  const interrupted = filtered.filter((s) => s.status === "interrupted" && !needsYou(s)).sort(order);
  const failed = filtered.filter((s) => s.status === "error" && !needsYou(s)).sort(order);
  const inTerminal = filtered.filter((s) => s.status === "in_terminal").sort(order);
  const archived = filtered.filter((s) => s.status === "closed").sort(order);
  const now = Date.now();
  const oldDone = done.filter((s) => !pinned.includes(s.id) && now - s.status_since > 3 * DAY);
  const cronNext = earliest(filteredCrons.filter((c) => !c.paused).map((c) => c.next_run_at));
  const scheduleNext = earliest(filteredSchedules.filter((l) => l.status === "queued").map((l) => l.ready_at));

  const archiveOld = async () => {
    setArchiving(true);
    try { for (const s of oldDone) await api.post(`/api/sessions/${s.id}/close`); } catch (e) { toastError(e); } finally { setArchiving(false); }
  };

  // Archiving the shown session returns to the dashboard without reading another run.
  const lastShown = useRef<{ id: string; status: Session["status"] } | null>(null);
  useEffect(() => {
    const was = lastShown.current;
    lastShown.current = visible ? { id: visible.id, status: visible.status } : null;
    if (visible && was?.id === visible.id && was.status !== "closed" && visible.status === "closed") {
      clearSession(visible.id);
    }
  }, [visible, clearSession]);

  type Item = { kind: "group"; title: AgentGroup; count: number; highlight?: boolean; detail?: string } | { kind: "session"; session: Session } | { kind: "schedule"; launch: ScheduledLaunch } | { kind: "cron"; cron: CronAgent } | { kind: "more" } | { kind: "archive-old" };
  // Groups with no items are not shown at all.
  const group = (title: AgentGroup, rows: Item[], opts: { highlight?: boolean; detail?: string; tail?: Item[] } = {}): Item[] => rows.length ? [{ kind: "group", title, count: rows.length, highlight: opts.highlight, detail: opts.detail }, ...(isCollapsed(collapsedGroups, title) ? [] : [...rows, ...(opts.tail ?? [])])] : [];
  const sessionRows = (list: Session[]) => list.map((session) => ({ kind: "session" as const, session }));
  const items: Item[] = [
    ...group("Needs you", sessionRows(needs), { highlight: true }),
    ...group("Running", sessionRows(running)),
    ...group("Quota paused", sessionRows(quotaPaused)),
    ...group("Done", sessionRows(done), { tail: oldDone.length ? [{ kind: "archive-old" }] : [] }),
    ...group("Stopped", sessionRows(stopped)),
    ...group("Interrupted", sessionRows(interrupted)),
    ...group("Failed", sessionRows(failed)),
    ...group("In terminal", sessionRows(inTerminal)),
    ...group("Cron", filteredCrons.map((cron) => ({ kind: "cron" as const, cron })), { detail: cronNext ? `next ${formatReset(cronNext / 1000)}` : undefined }),
    ...group("Scheduled", filteredSchedules.map((launch) => ({ kind: "schedule" as const, launch })), { detail: scheduleNext ? `next ${formatReset(scheduleNext / 1000)}` : undefined }),
    ...group("Archived", sessionRows(archived.slice(0, archivedLimit)), { tail: archived.length > archivedLimit ? [{ kind: "more" }] : [] }).map((i) => (i.kind === "group" ? { ...i, count: archived.length } : i)),
  ];
  const size = (i: Item) => (i.kind === "session" || i.kind === "schedule" || i.kind === "cron" ? 68 : i.kind === "archive-old" ? 44 : 34);
  const key = (i: Item) => (i.kind === "session" ? i.session.id : i.kind === "schedule" ? i.launch.id : i.kind === "cron" ? `cron-${i.cron.id}` : i.kind === "group" ? `group-${i.title}` : i.kind);
  const virtual = useVirtualizer({ count: items.length, getScrollElement: () => listRef.current, estimateSize: (i) => size(items[i]), getItemKey: (i) => key(items[i]), overscan: 6, useFlushSync: false });
  const name = (s: Session) => projectName(projects, s.project_id, s.cwd);
  const row = (s: Session) => <SessionRow key={s.id} s={s} project={name(s)} pinned={pinned.includes(s.id)} selected={s.id === visibleSession} onClick={() => open(s.id)} onMarkUnread={() => { if (s.id === visibleSession) clearSession(s.id); }} />;

  return (
    <div className="flex h-full min-h-0">
      <div className="flex w-[280px] flex-none flex-col border-r border-line bg-paper">
        <div className="grid gap-2 border-b border-line p-2">
          <input aria-label="Search sessions" className="field-input w-full" placeholder="Search sessions and messages…" value={filter} onChange={(e) => { setFilter(e.target.value); setArchivedLimit(30); }} />
          <div className="flex gap-2"><button className={`btn btn-sm ${pinsOnly ? "bg-teal-soft!" : ""}`} aria-pressed={pinsOnly} onClick={() => setPinsOnly(!pinsOnly)}>★ Pinned</button></div>
          {matches.isFetching && <span className="text-xs text-ink-3">Searching messages…</span>}
          {matches.error && <span className="text-xs text-danger">{matches.error.message}</span>}
        </div>
        <div ref={listRef} className="min-h-0 flex-1 overflow-x-hidden overflow-y-auto">
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
              {item.kind === "session" ? row(item.session) : item.kind === "cron" ? <CronRow cron={item.cron} selected={cronId === item.cron.id} onClick={() => setCronId(item.cron.id)} /> : item.kind === "schedule" ? <button onClick={() => setScheduledId(item.launch.id)} className={`grid h-[68px] w-full gap-1 border-b border-line px-3 py-2 text-left hover:bg-chrome ${scheduledId === item.launch.id ? "bg-select" : ""}`}><span className="truncate text-xs font-semibold">{item.launch.title}</span><span className="truncate text-[11px] text-ink-3" title={conditionsSummary(item.launch, sessions, schedules)}>{item.launch.status === "failed" ? "Needs attention · " : item.launch.status === "starting" ? "Starting · " : ""}{conditionsSummary(item.launch, sessions, schedules)}</span></button> : item.kind === "group" ? <Group title={item.title} count={item.count} highlight={item.highlight} detail={item.detail} collapsed={isCollapsed(collapsedGroups, item.title)} onToggle={() => toggleGroup(item.title)} /> :
                item.kind === "archive-old" ? <div className="flex h-full min-w-0 items-center px-3"><button className="btn btn-sm min-w-0 truncate" disabled={archiving} title="Archive seen Done sessions older than 3 days. Archived sessions stay resumable in Archived. Pinned sessions are kept." onClick={() => void archiveOld()}><Icon name="archive" size={13} />{archiving ? "Archiving…" : `Archive ${oldDone.length} seen, older than 3 d`}</button></div> :
                <button className="px-3 py-2 text-left text-xs text-teal-ink hover:underline" onClick={() => setArchivedLimit((n) => n + 50)}>Show more</button>}
            </div>;
          })}
        </div>
        </div>
      </div>
      <div className="min-h-0 min-w-0 flex-1 bg-paper">
        {cron ? (
          <CronPane key={cron.id} cron={cron} onClose={() => setCronId(null)} onOpen={(s) => open(s.id)} />
        ) : scheduled ? (
          <ScheduledPane key={scheduled.id} launch={scheduled} onClose={clearSide} onStart={(s) => open(s.id)} />
        ) : visibleSession ? (
          <SessionPane key={`${scope.workspace?.id ?? "all"}:${visibleSession}:${panelLoad}`} sessionId={visibleSession} onClear={() => clearSession(visibleSession)} />
        ) : (
          <AgentsHome sessions={sessions} crons={crons} schedules={schedules} name={name} onOpen={open} onCron={setCronId} onSchedule={setScheduledId} />
        )}
      </div>
    </div>
  );
}

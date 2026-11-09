// Agents: a monitor of what runs and what ran, grouped by state, showing one session at a time. What needs you is
// in Today; unseen sessions stay bold in their group. Clicking a session opens its conversation. The window's
// workspace narrows every group.

import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import { Empty, MoreMenu } from "@/components/ui";
import { ActionMenuItem } from "@/components/ActionButton";
import { MenuItem } from "@/components/Popover";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useEffect, useRef, useState } from "react";
import type { Session } from "@/gen/Session";
import { HarnessMark } from "@/components/HarnessMark";
import { isRunning, isDone, isQuotaPaused, isStopped, isUnseen, needsYou, sessionPlace } from "@/lib/sessionStatus";
import { Icon } from "@/components/Icon";
import { LaunchButton } from "@/features/launch/LaunchButton";
import { projectName, useProjects, useSession, useSessions } from "@/lib/queries";
import { useScope } from "@/lib/workspace";
import { isNoUpdate } from "@/lib/routines";
import { sendPresence } from "@/lib/ws";
import { isCollapsed, useLayout, type AgentGroup } from "@/stores/layout";
import { SessionPane } from "./SessionPane";
import { SessionStatus } from "./SessionBits";
import { useMarkSeen, useWindowFocused } from "./useSeen";
import { age, formatReset } from "@/lib/format";
import { toastError } from "@/components/Toast";
import { usePhone } from "@/lib/phone";

/** Statuses whose badge tells more than the group heading. */
const informative = (s: Session) => s.open_question || s.status === "error" || s.status === "interrupted" || s.status === "quota_paused";

const SessionRow = memo(function SessionRow({ s, project, selected, pinned, onClick, onMarkUnread }: { s: Session; project: string; selected: boolean; pinned: boolean; onClick: () => void; onMarkUnread: () => void }) {
  const dim = s.status === "closed";
  const stop = isRunning(s) || isQuotaPaused(s);
  const canMarkUnread = !isUnseen(s) && (isDone(s) || s.status === "error" || s.status === "interrupted");
  const needs = needsYou(s);
  const blockers = s.quota_pause?.blockers ?? [];
  const quotaDetail = s.quota_pause?.error ?? blockers.map((b) => `${b.window} reset ${b.resets_at === null ? "unknown" : formatReset(b.resets_at)}`).join(" · ");
  // One short phrase in the row (the latest reset); every window in the tooltip.
  const lastReset = blockers.length && blockers.every((b) => b.resets_at !== null) ? Math.max(...blockers.map((b) => b.resets_at!)) : null;
  const quotaLine = s.quota_pause?.error ?? (lastReset !== null ? `resets ${formatReset(lastReset)}` : quotaDetail);
  const line = s.quota_pause ? quotaLine || "Waiting for a quota update" : isNoUpdate(s.last_message) ? "No update (a routine with nothing to report)" : s.last_message ?? (s.usage.cost_usd != null ? `$${s.usage.cost_usd.toFixed(2)}` : "");
  const full = s.quota_pause ? quotaDetail || line : line;
  return (
    <div className="group relative" data-tour="agents.row" data-tour-id={s.id}>
      <button
        type="button"
        onClick={onClick}
        className={`grid h-[68px] w-full grid-cols-[18px_1fr_auto] content-start gap-x-[9px] gap-y-[3px] border-b border-line-2 px-3 py-2 text-left hover:bg-chrome ${selected ? "bg-select hover:bg-select" : ""}`}
      >
        <span className="row-span-2 mt-px">
          <HarnessMark harness={s.harness} size={18} />
        </span>
        <span className={`truncate text-[13px] leading-[1.35] ${dim ? "font-medium text-ink-2" : needs ? "font-semibold" : "font-medium"}`} title={s.title}>
          {pinned ? "★ " : ""}{s.title}
        </span>
        <span className="flex items-center gap-1 text-[12px] whitespace-nowrap text-ink-3">
          {informative(s) ? <SessionStatus s={s} /> : <>{isRunning(s) && <span className="lp-spin" title="Running" />}{age(dim ? s.ended_at ?? s.status_since : s.status_since)}</>}
        </span>
        <span className="agent-row-line col-span-2 line-clamp-2 pr-8 text-[12px] leading-[1.3] text-ink-3" title={full ? `${project} · ${full}` : project}>
          <span className="text-ink-2">{sessionPlace(s, project)}</span>{line && <> · <span className={s.quota_pause ? "text-attn" : ""}>{line}</span></>}
        </span>
      </button>
      {/* The row opens the session; Stop, Archive, Mark unread and Pin in ⋯. */}
      <span className="agent-row-action lp-row-more absolute right-1.5 bottom-1.5">
        <MoreMenu label={`More actions for ${s.title}`}>{(close) => <>
          {stop && <ActionMenuItem name="session.stop" params={{ id: s.id }} close={close} />}
          {!stop && !dim && <ActionMenuItem name="session.archive" params={{ id: s.id }} close={close} detail="It stays resumable" />}
          {canMarkUnread && <ActionMenuItem name="session.unread" params={{ id: s.id }} close={() => { close(); onMarkUnread(); }} label="Mark unread" detail="Return to Today for later" icon="mail" />}
          <MenuItem icon={<span className="text-teal-ink">{pinned ? "★" : "☆"}</span>} label={pinned ? "Unpin" : "Pin"} detail="Pinned sessions show first" onClick={() => { close(); useLayout.getState().togglePin(s.id); }} />
        </>}</MoreMenu>
      </span>
    </div>
  );
});

function Group({ title, count, highlight, detail, collapsed, onToggle }: { title: AgentGroup; count: number; highlight?: boolean; detail?: string; collapsed: boolean; onToggle: () => void }) {
  return (
    <button
      type="button"
      aria-expanded={!collapsed}
      aria-label={`${collapsed ? "Expand" : "Collapse"} ${title}`}
      data-tour="agents.group"
      data-tour-id={title}
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
  const projects = useProjects().data;
  const { selectedSession, show, clearSession, collapsedGroups, toggleGroup } = useLayout();
  const search = useSearch({ strict: false }) as { session?: string; view?: "changes" };
  const navigate = useNavigate();
  // The phone shows the list or one session at a time; the address holds the session, so Back returns to the list.
  const phone = usePhone();
  const [archivedLimit, setArchivedLimit] = useState(30);
  const listRef = useRef<HTMLDivElement>(null);
  const open = (id: string) => {
    show(id);
    setPanelLoad((n) => n + 1);
    if (phone) void navigate({ to: "/agents", search: { session: id } });
  };
  const back = () => {
    void navigate({ to: "/agents", search: {} });
  };
  const [initialView, setInitialView] = useState<"changes" | undefined>(undefined);

  // From a notification or another page: /agents?session=<id>[&view=changes]. The desktop then clears the address;
  // the phone keeps it and reopens the pane when `view` changes (the pane key holds it).
  useEffect(() => {
    if (search.session && !phone) {
      show(search.session);
      setInitialView(search.view);
      setPanelLoad((n) => n + 1);
      void navigate({ to: "/agents", search: {}, replace: true });
    }
  }, [search.session, show, navigate]);

  // Tell the daemon which sessions are on screen, so it does not notify about them, and mark the shown session seen.
  const focused = useWindowFocused();
  const wanted = phone ? search.session ?? null : selectedSession;
  const selected = useSession(wanted);
  const visible = selected && sessions.some((s) => s.id === wanted) ? selected : undefined;
  const visibleSession = visible?.id ?? null;
  useEffect(() => { sendPresence("agents", focused, visibleSession ? [visibleSession] : []); }, [focused, visibleSession]);
  useEffect(() => () => sendPresence("agents", false, []), []);
  useMarkSeen(visible, focused);

  const needle = filter.trim().toLocaleLowerCase();
  const filtered = sessions.filter((s) => (!pinsOnly || pinned.includes(s.id)) && (!needle || [s.title, s.cwd, s.prompt, s.last_message ?? "", projectName(projects, s.project_id, s.cwd)].some((t) => t.toLocaleLowerCase().includes(needle)) || (debounced === filter.trim() && matches.data?.includes(s.id))));
  const order = (a: Session, b: Session) => b.started_at - a.started_at;
  const newest = (a: Session, b: Session) => b.status_since - a.status_since;
  const local = filtered;
  const done = local.filter(isDone).sort(newest);
  const running = local.filter((s) => isRunning(s)).sort(order);
  const quotaPaused = local.filter(isQuotaPaused).sort(order);
  const stopped = local.filter(isStopped).sort(order);
  const interrupted = local.filter((s) => s.status === "interrupted").sort(order);
  const failed = local.filter((s) => s.status === "error").sort(order);
  const inTerminal = local.filter((s) => s.status === "in_terminal").sort(order);
  const archived = local.filter((s) => s.status === "closed").sort(order);
  const now = Date.now();
  const oldDone = done.filter((s) => !needsYou(s) && !pinned.includes(s.id) && now - s.status_since > 3 * DAY);

  const archiveOld = async () => {
    setArchiving(true);
    try { for (const s of oldDone) await api.post(`/api/sessions/${s.id}/close`); } catch (e) { toastError(e); } finally { setArchiving(false); }
  };

  // Archiving the shown session returns to the empty pane without reading another run.
  const lastShown = useRef<{ id: string; status: Session["status"] } | null>(null);
  useEffect(() => {
    const was = lastShown.current;
    lastShown.current = visible ? { id: visible.id, status: visible.status } : null;
    if (visible && was?.id === visible.id && was.status !== "closed" && visible.status === "closed") {
      if (phone) back(); else clearSession(visible.id);
    }
  }, [visible, clearSession]);

  type Item = { kind: "group"; title: AgentGroup; count: number; highlight?: boolean; detail?: string } | { kind: "session"; session: Session } | { kind: "more" } | { kind: "archive-old" };
  const groupCollapsed = (title: AgentGroup) => isCollapsed(collapsedGroups, title);
  const toggleListGroup = toggleGroup;
  // Groups with no items are not shown at all.
  const group = (title: AgentGroup, rows: Item[], opts: { highlight?: boolean; detail?: string; tail?: Item[] } = {}): Item[] => rows.length ? [{ kind: "group", title, count: rows.length, highlight: opts.highlight, detail: opts.detail }, ...(groupCollapsed(title) ? [] : [...rows, ...(opts.tail ?? [])])] : [];
  const sessionRows = (list: Session[]) => list.map((session) => ({ kind: "session" as const, session }));
  const items: Item[] = [
    ...group("Running", sessionRows(running)),
    ...group("Quota paused", sessionRows(quotaPaused)),
    ...group("Done", sessionRows(done), { tail: oldDone.length ? [{ kind: "archive-old" }] : [] }),
    ...group("Stopped", sessionRows(stopped)),
    ...group("Interrupted", sessionRows(interrupted)),
    ...group("Failed", sessionRows(failed)),
    ...group("In terminal", sessionRows(inTerminal)),
    ...group("Archived", sessionRows(archived.slice(0, archivedLimit)), { tail: archived.length > archivedLimit ? [{ kind: "more" }] : [] }).map((i) => (i.kind === "group" ? { ...i, count: archived.length } : i)),
  ];
  const size = (i: Item) => (i.kind === "session" ? 68 : i.kind === "archive-old" ? 44 : 34);
  const key = (i: Item) => (i.kind === "session" ? i.session.id : i.kind === "group" ? `group-${i.title}` : i.kind);
  const virtual = useVirtualizer({ count: items.length, getScrollElement: () => listRef.current, estimateSize: (i) => size(items[i]), getItemKey: (i) => key(items[i]), overscan: 6, useFlushSync: false });
  const name = (s: Session) => projectName(projects, s.project_id, s.cwd);
  const row = (s: Session) => <SessionRow key={s.id} s={s} project={name(s)} pinned={pinned.includes(s.id)} selected={s.id === visibleSession} onClick={() => open(s.id)} onMarkUnread={() => { if (s.id === visibleSession) clearSession(s.id); }} />;

  const paneOpen = !!visibleSession;
  return (
    <div className="flex h-full min-h-0">
      <div className={`flex flex-col bg-paper ${phone ? (paneOpen ? "hidden" : "w-full") : "w-[280px] flex-none border-r border-line"}`}>
        <div className="grid gap-2 border-b border-line p-2">
          <input aria-label="Search sessions" className="field-input w-full" placeholder="Search sessions and messages…" value={filter} onChange={(e) => { setFilter(e.target.value); setArchivedLimit(30); }} />
          <div className="flex flex-wrap items-center gap-2"><button className="lp-chip" aria-pressed={pinsOnly} onClick={() => setPinsOnly(!pinsOnly)}>★ Pinned</button>{phone && <span className="ml-auto"><LaunchButton small title="Open the composer (the draft if one exists)" /></span>}</div>
          {matches.isFetching && <span className="text-xs text-ink-3">Searching messages…</span>}
          {matches.error && <span className="text-xs text-danger-ink">{matches.error.message}</span>}
        </div>
        <div ref={listRef} data-tour="agents.list" className="min-h-0 flex-1 overflow-x-hidden overflow-y-auto">
        {sessions.length === 0 && (
          <div className="grid gap-3 p-4 text-[13px] text-ink-2">
            {`No agent in ${scope.workspace.name} yet.`}
            <span>
              <LaunchButton small />
            </span>
          </div>
        )}
        <div style={{ height: virtual.getTotalSize(), position: "relative", flexShrink: 0 }}>
          {virtual.getVirtualItems().map((v) => {
            const item = items[v.index];
            return <div key={v.key} style={{ position: "absolute", top: 0, left: 0, width: "100%", height: v.size, transform: `translateY(${v.start}px)` }}>
              {item.kind === "session" ? row(item.session) : item.kind === "group" ? <Group title={item.title} count={item.count} highlight={item.highlight} detail={item.detail} collapsed={groupCollapsed(item.title)} onToggle={() => toggleListGroup(item.title)} /> :
                item.kind === "archive-old" ? <div className="flex h-full min-w-0 items-center px-3"><button className="btn btn-sm min-w-0 truncate" disabled={archiving} title="Archive seen Done sessions older than 3 days. Archived sessions stay resumable in Archived. Pinned sessions are kept." onClick={() => void archiveOld()}><Icon name="archive" size={13} />{archiving ? "Archiving…" : `Archive ${oldDone.length} seen, older than 3 d`}</button></div> :
                <button className="px-3 py-2 text-left text-xs text-teal-ink hover:underline" onClick={() => setArchivedLimit((n) => n + 50)}>Show more</button>}
            </div>;
          })}
        </div>
        </div>
      </div>
      <div className={`min-h-0 min-w-0 flex-1 bg-paper ${phone && !paneOpen ? "hidden" : ""}`}>
        {visibleSession ? (
          <SessionPane key={`${scope.workspace.id}:${visibleSession}:${panelLoad}${phone ? `:${search.view ?? ""}` : ""}`} sessionId={visibleSession} initialView={phone ? search.view : initialView} onClear={() => (phone ? back() : clearSession(visibleSession))} />
        ) : phone ? null : (
          <Empty fill icon="agents" action={<LaunchButton small />}>Pick an agent on the left. Questions and finished agents wait in <Link to="/" className="text-teal-ink">Today</Link>.</Empty>
        )}
      </div>
    </div>
  );
}

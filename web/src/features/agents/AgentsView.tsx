import { useTopics, useTopicMutations } from "@/features/topics/data";
import { TopicPicker } from "@/features/topics/TopicPicker";
// Agents: a monitor of what runs and what ran, grouped by state, showing one session at a time. What needs you is
// in Today; unseen sessions stay bold in their group. Clicking a session opens its conversation. The window's
// workspace narrows every group.

import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { openExternal } from "@/lib/platform";
import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import { BotChip, ConfirmButton, Empty, MoreMenu } from "@/components/ui";
import { ActionMenuItem } from "@/components/ActionButton";
import { MenuItem } from "@/components/Popover";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Session } from "@/gen/Session";
import type { Predraft } from "@/gen/Predraft";
import type { WorktreeInfo } from "@/gen/WorktreeInfo";
import { workTags, WorkTagPill, type WorkTag } from "./workTag";
import { HarnessMark } from "@/components/HarnessMark";
import { isRunning, isDone, isQuotaPaused, isStopped, isUnseen, needsYou } from "@/lib/sessionStatus";
import { Icon } from "@/components/Icon";
import { LaunchButton } from "@/features/launch/LaunchButton";
import { projectName, useProjects, useSession, useSessions } from "@/lib/queries";
import { useScope } from "@/lib/workspace";
import { isNoUpdate } from "@/lib/routines";
import { sendPresence } from "@/lib/ws";
import { isCollapsed, useLayout, type AgentGroup } from "@/stores/layout";
import { SessionPane, type View } from "./SessionPane";
import { phoneBack } from "@/mobile/back";
import { PullToRefresh } from "@/components/PullToRefresh";
import { useQueryClient } from "@tanstack/react-query";
import { SessionStatus } from "./SessionBits";
import { useMarkSeen, useWindowFocused } from "./useSeen";
import { age, formatReset } from "@/lib/format";
import { toast, toastError } from "@/components/Toast";
import { runAction } from "@/lib/actions";
import { isPhone, usePhone } from "@/lib/phone";

/** Statuses whose badge tells more than the group heading. */
const informative = (s: Session) => s.open_question || s.status === "error" || s.status === "interrupted" || s.status === "quota_paused";

/** Row height: the phone's larger text needs more room for the two lines of the last message. */
const ROW = isPhone ? 80 : 68;

/** The lower-cased text the search box matches, computed once per session record (records are replaced on update). */
const searchTexts = new WeakMap<Session, string>();
function searchText(s: Session): string {
  let text = searchTexts.get(s);
  if (text === undefined) { text = [s.title, s.cwd, s.prompt, s.last_message ?? ""].join("\n").toLocaleLowerCase(); searchTexts.set(s, text); }
  return text;
}

/** Rows take stable callbacks with the session id, so a bus update re-renders only the rows whose session changed. */
const SessionRow = memo(function SessionRow({ s, project, selected, pinned, tag, onOpen, onMarkUnread }: { s: Session; project: string; selected: boolean; pinned: boolean; tag?: WorkTag; onOpen: (id: string) => void; onMarkUnread: (id: string) => void }) {
  const navigate = useNavigate();
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
        onClick={() => onOpen(s.id)}
        style={{ height: ROW }} className={`grid w-full grid-cols-[18px_1fr_auto] content-start gap-x-2 gap-y-1 border-b border-line-2 px-3 py-2 text-left hover:bg-chrome ${selected ? "bg-select hover:bg-select" : ""}`}
      >
        <span className="row-span-2 mt-px">
          <HarnessMark harness={s.harness} size={18} />
        </span>
        <span className={`flex min-w-0 items-center gap-1.5 text-[13px] leading-[1.35] ${dim ? "font-medium text-ink-2" : needs ? "font-semibold" : "font-medium"}`} title={s.title}>
          {s.launch_spec.bot && <BotChip name={s.launch_spec.bot.name} />}
          <span className="min-w-0 truncate">{pinned ? "★ " : ""}{s.title}</span>
        </span>
        <span className="flex items-center gap-1 text-[12px] whitespace-nowrap text-ink-3">
          {informative(s) ? <SessionStatus s={s} /> : <>{isRunning(s) && <span className="lp-spin" title="Running" />}{age(dim ? s.ended_at ?? s.status_since : s.status_since)}</>}
        </span>
        <span className="agent-row-line col-span-2 line-clamp-2 pr-8 text-[12px] leading-[1.3] text-ink-3" title={full ? `${project} · ${full}` : project}>
          {tag && <WorkTagPill tag={tag} className="mr-1.5 align-[1px]" />}<span className="text-ink-2">{project}</span>{line && <> · <span className={s.quota_pause ? "text-attn" : ""}>{line}</span></>}
        </span>
      </button>
      {/* The row opens the session; Stop, Archive, Mark unread and Pin in ⋯. */}
      <span className="agent-row-action lp-row-more absolute right-1.5 bottom-1.5">
        <MoreMenu label={`More actions for ${s.title}`}>{(close) => <>
          {stop && <ActionMenuItem name="session.stop" params={{ id: s.id }} close={close} />}
          {!stop && !dim && <ActionMenuItem name="session.archive" params={{ id: s.id }} close={close} detail="It stays resumable" />}
          {tag?.pr && <MenuItem icon={<Icon name="pr" size={14} />} label={`Open PR #${tag.pr.number}`} detail={tag.pr.repo} onClick={() => { close(); openExternal(`https://github.com/${tag.pr!.repo}/pull/${tag.pr!.number}`); }} />}
          {tag?.localPr && <MenuItem icon={<Icon name="pr-local" size={14} />} label="Open Local PR" detail={tag.localPr.title} onClick={() => { close(); void navigate({ to: "/predrafts/$id", params: { id: tag.localPr!.id } }); }} />}
          {canMarkUnread && <ActionMenuItem name="session.unread" params={{ id: s.id }} close={() => { close(); onMarkUnread(s.id); }} label="Mark unread" detail="Return to Today for later" icon="mail" />}
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
  const qc = useQueryClient();
  const [panelLoad, setPanelLoad] = useState(0);
  const [filter, setFilter] = useState("");
  const [debounced, setDebounced] = useState("");
  const topicsQuery = useTopics();
  const topics = topicsQuery.data ?? [];
  const { add: addTopic } = useTopicMutations();
  const [selection, setSelection] = useState<string[]>([]);
  const [selecting, setSelecting] = useState(false);
  const [pinsOnly, setPinsOnly] = useState(false);
  const [archiving, setArchiving] = useState(false);
  const pinned = useLayout((s) => s.pinned);
  useEffect(() => { const t = setTimeout(() => setDebounced(filter.trim()), 250); return () => clearTimeout(t); }, [filter]);
  const matches = useQuery({ queryKey: ["session-search", debounced], queryFn: () => api.get<string[]>(`/api/sessions/search?q=${encodeURIComponent(debounced)}`), enabled: !!debounced });
  const projects = useProjects().data;
  // Where each agent's work went (merged, in a Local PR or pull request, read-only): a tag in its row.
  const worktrees = useQuery({ queryKey: ["worktrees", "all"], queryFn: () => api.get<WorktreeInfo[]>("/api/worktrees"), staleTime: 30_000 }).data;
  const localPrs = useQuery({ queryKey: ["predrafts"], queryFn: () => api.get<Predraft[]>("/api/predrafts") }).data;
  const tags = useMemo(() => workTags(sessionsQuery.data ?? [], worktrees, localPrs), [sessionsQuery.data, worktrees, localPrs]);
  const { selectedSession, show, clearSession, collapsedGroups, toggleGroup } = useLayout();
  const search = useSearch({ strict: false }) as { session?: string; view?: View; topic?: string };
  const navigate = useNavigate();
  useEffect(() => {
    setSelection([]); setSelecting(false);
  }, [scope.workspace.id]);
  useEffect(() => {
    if (search.topic && topicsQuery.data && !topicsQuery.data.some((t) => t.id === search.topic)) void navigate({ to: "/agents", search: { ...search, topic: undefined }, replace: true });
  }, [search.topic, topicsQuery.data, navigate]);
  // The phone shows the list or one session at a time; the address holds the session, so Back returns to the list.
  const phone = usePhone();
  const [archivedLimit, setArchivedLimit] = useState(30);
  const listRef = useRef<HTMLDivElement>(null);
  // A click in the list opens the session on its default view (a `view` from a link applied only to that session);
  // clicking the row of the open session keeps its pane as it is (tab, scroll, trace position).
  const openRef = useRef<(id: string) => void>(() => {});
  const open = useCallback((id: string) => openRef.current(id), []);
  // ‹, Archive and Mark as unread: one step back (to Today when the agent came from there), never a new entry.
  const back = () => phoneBack("/agents");
  const [initialView, setInitialView] = useState<View | undefined>(undefined);

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
  openRef.current = (id: string) => {
    if (phone) { void navigate({ to: "/agents", search: { session: id } }); show(id); return; }
    if (id === visibleSession) return;
    setInitialView(undefined);
    show(id);
  };

  const needle = filter.trim().toLocaleLowerCase();
  const filtered = sessions.filter((s) => (!search.topic || topics.some((t) => t.id === search.topic && t.session_ids.includes(s.id))) && (!pinsOnly || pinned.includes(s.id)) && (!needle || searchText(s).includes(needle) || projectName(projects, s.project_id, s.cwd).toLocaleLowerCase().includes(needle) || (debounced === filter.trim() && matches.data?.includes(s.id))));
  const order = (a: Session, b: Session) => b.started_at - a.started_at;
  const newest = (a: Session, b: Session) => b.status_since - a.status_since;
  // What needs you (a question, an unseen Done, Failed or Interrupted) comes first, out of its state's group.
  const needs = filtered.filter((s) => s.status !== "closed" && needsYou(s)).sort(newest);
  const needIds = new Set(needs.map((s) => s.id));
  const local = filtered.filter((s) => !needIds.has(s.id));
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

  // Archives each one and says how many it archived; one failure does not stop the others.
  const archiveOld = async () => {
    setArchiving(true);
    const list = oldDone;
    let archivedCount = 0;
    const errors: string[] = [];
    try {
      for (const s of list) {
        try { await runAction("session.archive", { id: s.id }); archivedCount += 1; }
        catch (e) { errors.push(e instanceof Error ? e.message : String(e)); }
      }
      await qc.invalidateQueries({ queryKey: ["sessions"] });
    } finally { setArchiving(false); }
    const what = `Archived ${archivedCount} of ${list.length} session${list.length === 1 ? "" : "s"}`;
    if (errors.length) toastError(new Error(`${what}. ${errors.length} failed: ${errors[0]}`));
    else toast(`Archived ${archivedCount} session${archivedCount === 1 ? "" : "s"}. They stay resumable in Archived.`);
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
    ...group("Needs you", sessionRows(needs), { highlight: true }),
    ...group("Running", sessionRows(running)),
    ...group("Quota paused", sessionRows(quotaPaused)),
    ...group("Done", sessionRows(done), { tail: oldDone.length ? [{ kind: "archive-old" }] : [] }),
    ...group("Stopped", sessionRows(stopped)),
    ...group("Interrupted", sessionRows(interrupted)),
    ...group("Failed", sessionRows(failed)),
    ...group("In terminal", sessionRows(inTerminal)),
    ...group("Archived", sessionRows(archived.slice(0, archivedLimit)), { tail: archived.length > archivedLimit ? [{ kind: "more" }] : [] }).map((i) => (i.kind === "group" ? { ...i, count: archived.length } : i)),
  ];
  const size = (i: Item) => (i.kind === "session" ? ROW : i.kind === "archive-old" ? 44 : 34);
  const key = (i: Item) => (i.kind === "session" ? i.session.id : i.kind === "group" ? `group-${i.title}` : i.kind);
  const virtual = useVirtualizer({ count: items.length, getScrollElement: () => listRef.current, estimateSize: (i) => size(items[i]), getItemKey: (i) => key(items[i]), overscan: 6, useFlushSync: false });
  const name = (s: Session) => projectName(projects, s.project_id, s.cwd);
  const markUnreadRef = useRef<(id: string) => void>(() => {});
  markUnreadRef.current = (id) => { if (id === visibleSession) clearSession(id); };
  const markUnread = useCallback((id: string) => markUnreadRef.current(id), []);
  const row = (s: Session) => <div className="flex items-center">{selecting && <label className="flex shrink-0 items-center justify-center px-2" style={{ minHeight: ROW }}><input type="checkbox" aria-label={`Select ${s.title}`} checked={selection.includes(s.id)} onChange={(e) => setSelection((ids) => e.target.checked ? [...ids, s.id] : ids.filter((id) => id !== s.id))} /></label>}<div className="min-w-0 flex-1"><SessionRow key={s.id} s={s} project={name(s)} pinned={pinned.includes(s.id)} tag={tags.get(s.id)} selected={s.id === visibleSession} onOpen={open} onMarkUnread={markUnread} /></div></div>;

  const paneOpen = !!visibleSession;
  return (
    <div className="flex h-full min-h-0">
      <div className={`flex flex-col bg-paper ${phone ? (paneOpen ? "hidden" : "w-full") : "w-[280px] flex-none border-r border-line"}`}>
        <div className="grid gap-2 border-b border-line p-2">
          <input aria-label="Search sessions" className="field-input w-full" placeholder="Search sessions and messages…" value={filter} onChange={(e) => { setFilter(e.target.value); setArchivedLimit(30); }} />
          <div className="flex flex-wrap items-center gap-2"><button className="lp-chip" aria-pressed={pinsOnly} onClick={() => setPinsOnly(!pinsOnly)}>★ Pinned</button>{phone && <span className="ml-auto"><LaunchButton small title="Open the composer (the draft if one exists)" /></span>}</div>
          <div className="flex flex-wrap items-center gap-2"><select className="field-input min-w-0 flex-1" aria-label="Filter by topic" value={search.topic ?? ""} onChange={(e) => void navigate({ to: "/agents", search: { ...search, topic: e.target.value || undefined }, replace: true })}><option value="">All topics</option>{topics.map((t) => <option key={t.id} value={t.id}>{t.name}</option>)}</select><Link to="/topics" className="btn btn-sm">Topics</Link></div>
          <div className="flex flex-wrap items-center gap-2"><button className="btn btn-sm" onClick={() => { setSelecting(!selecting); setSelection([]); }}>{selecting ? "Cancel selection" : "Select sessions"}</button>{selecting && selection.length > 0 && <TopicPicker label={`Add ${selection.length} to topic…`} onPick={async (t) => { await addTopic(t.id, selection); setSelection([]); setSelecting(false); }} />}</div>
          {matches.isFetching && <span className="text-[12px] text-ink-3">Searching messages…</span>}
          {matches.error && <span className="text-[12px] text-danger-ink">{matches.error.message}</span>}
        </div>
        <div ref={listRef} data-tour="agents.list" className="min-h-0 flex-1 overflow-x-hidden overflow-y-auto">
        {phone && <PullToRefresh target={listRef} onRefresh={() => qc.invalidateQueries({ queryKey: ["sessions"] })} />}
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
                item.kind === "archive-old" ? <div className="flex h-full min-w-0 items-center px-3"><ConfirmButton className="min-w-0 truncate" icon="archive" disabled={archiving} title="Archive seen Done sessions older than 3 days. Archived sessions stay resumable in Archived. Pinned sessions are kept." consequence={`Archive ${oldDone.length}?`} confirmLabel="Archive" onConfirm={archiveOld}>{archiving ? "Archiving…" : `Archive ${oldDone.length} seen, older than 3 d`}</ConfirmButton></div> :
                <button className="px-3 py-2 text-left text-[12px] text-teal-ink hover:underline" onClick={() => setArchivedLimit((n) => n + 50)}>Show more</button>}
            </div>;
          })}
        </div>
        </div>
      </div>
      <div className={`min-h-0 min-w-0 flex-1 bg-paper ${phone && !paneOpen ? "hidden" : ""}`}>
        {visibleSession ? (
          <SessionPane key={`${scope.workspace.id}:${visibleSession}:${panelLoad}`} sessionId={visibleSession} initialView={phone ? search.view : initialView} onView={phone ? (view) => void navigate({ to: "/agents", search: { session: visibleSession, view }, replace: true }) : undefined} onClear={() => (phone ? back() : clearSession(visibleSession))} />
        ) : phone ? null : (
          <Empty fill icon="agents" action={<LaunchButton small />}>Pick an agent on the left. Questions and finished agents wait in <Link to="/" className="text-teal-ink">Today</Link>.</Empty>
        )}
      </div>
    </div>
  );
}

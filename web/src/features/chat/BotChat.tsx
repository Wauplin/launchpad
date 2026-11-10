// The chat of a bot: every bot page shows it (Ask Launchpad has no code of its own). It draws the sessions of the
// bot's thread from their events (chatRows.ts): your messages, the bot's answers, one steps line per turn, its
// questions to answer in place, and what each turn wrote (proposals, ready clicks, changes). The trace stays in
// Agents. Newest at the bottom; older events and sessions load as you scroll up, tail-first. It fills its parent,
// so it works as a page, a panel and the phone's full screen. The message box sends to the bot's thread.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import type { Bot } from "@/gen/Bot";
import type { Change } from "@/gen/Change";
import type { Proposal } from "@/gen/Proposal";
import type { QueuedMessage } from "@/gen/QueuedMessage";
import { ActionButton } from "@/components/ActionButton";
import type { Session } from "@/gen/Session";
import type { Suggestion } from "@/gen/Suggestion";
import { BotAvatar } from "@/components/BotAvatar";
import { useHarnessInfo } from "@/components/HarnessMark";
import { Icon } from "@/components/Icon";
import { toastError } from "@/components/Toast";
import { AskCard } from "@/features/agents/AskCard";
import { AssistantRow, Separator } from "@/features/agents/EventRow";
import { ATTACH_LIMITS, AttachmentList, useMessageAttachments } from "@/features/agents/MessageAttachments";
import { PresetChips, usePresets } from "@/features/agents/MessagePresets";
import { EMPTY_CONTEXT, hasMessageContext, MessageContextButton, MessageContextChips } from "@/features/agents/MessageContext";
import { withPresets } from "@/lib/presets";
import { senderLine } from "@/lib/senders";
import { instructionsUpdated } from "@/lib/bots";
import { useMarkSeen, useWindowFocused } from "@/features/agents/useSeen";
import { buildRows } from "@/features/agents/rows";
import { foldTrace } from "@/features/agents/traceRows";
import { ProposalCard, useProposalList } from "@/features/proposals/ProposalCard";
import { useSuggestionList } from "@/features/suggestions/data";
import { useToday, type TodayItem } from "@/features/today/data";
import { BranchActions, ItemCard, useBranch } from "@/features/today/TodayCards";
import { runAction } from "@/lib/actions";
import { ACTIONS } from "@/gen/actions";
import type { Activity } from "@/gen/Activity";
import { activityLine, activityLink, activityParts, useActivity } from "./activity";
import { api } from "@/lib/api";
import { pastedImages } from "@/lib/attachments";
import { daemonAsset } from "@/lib/connection";
import { age, ago, when } from "@/lib/format";
import { useCrons, useSession } from "@/lib/queries";
import { emptyLog, useSessionLogs, type SessionLog } from "@/lib/sessionStore";
import { isRunning } from "@/lib/sessionStatus";
import { sendPresence } from "@/lib/ws";
import { QuickChips } from "./QuickChips";
import { chatRows, stepsLabel, type ChatCard, type ChatItem, type ChatSession } from "./chatRows";

/** Today kinds that are a ready click in a bot's chat: Send, Post, Review & publish. */
const READY = new Set<TodayItem["kind"]>(["outbox", "suggestion", "local_pr"]);


// Display rows per log object: a session whose events did not change keeps its rows.
const folded = new WeakMap<SessionLog, ReturnType<typeof foldTrace>>();

/** What the thread's sessions changed: knowledge-base notes, and Launchpad records they changed directly at admin
 * access (the activity list). Changed lines at their turn. */
function useChanges(bot: Bot, ids: Set<string>): { cards: ChatCard[]; activity: Map<string, Activity | Change> } {
  const ws = bot.workspace_id;
  const kb = useQuery({ queryKey: ["knowledge", ws, "changes"], queryFn: () => api.get<{ changes: Change[] }>(`/api/knowledge/${ws}/changes`) }).data?.changes;
  const done = useActivity(ws).data;
  return useMemo(() => {
    const of = (c: Change) => c.author?.session_id ?? c.session_id;
    const mine = (done ?? []).filter((a) => ids.has(a.session_id));
    return {
      cards: [
        ...(kb ?? []).filter((c) => ids.has(of(c) ?? "")).map((c): ChatCard => ({ kind: "changed", id: `kb${c.id}`, session_id: of(c)!, at: c.at, v: "", text: kbLine(c) })),
        ...mine.map((a): ChatCard => ({ kind: "changed", id: `act${a.id}`, session_id: a.session_id, at: a.created_at, v: "", text: activityLine(a) })),
      ],
      activity: new Map<string, Activity | Change>([
        ...(kb ?? []).map((c): [string, Change] => [`kb${c.id}`, c]),
        ...mine.map((a): [string, Activity] => [`act${a.id}`, a]),
      ]),
    };
  }, [kb, done, ids]);
}

/** A new session the owner asked for (⋯ → Start a new session): the next message starts it, with their reason. */
export type NewSessionRequest = { reason: string };

export function BotChat({ bot, thread, rotation, needsProfile, openSettings, intro, view = "bot", newSession = null, onNewSession, onStep }: {
  bot: Bot;
  /** Which view shows the chat (a bot page, the Ask panel), for the sessions this window reports on screen. */
  view?: string;
  /** The bot's sessions, oldest first. */
  thread: Session[];
  /** Why the next message starts a new session anyway (profile or folder changed), or null. */
  rotation: string | null;
  needsProfile: boolean;
  openSettings: () => void;
  /** Shown while the thread is empty. */
  intro: React.ReactNode;
  /** The next message starts a new session (the bot page's ⋯); `onNewSession(null)` cancels it, and sending clears it. */
  newSession?: NewSessionRequest | null;
  onNewSession?: (r: NewSessionRequest | null) => void;
  /** The step the current turn works on (its last step title), for the page's status line. */
  onStep?: (step: string | null) => void;
}) {
  const current = [...thread].reverse().find((s) => s.status !== "closed") ?? null;
  // Sessions shown, counted from the newest; a new session of the thread adds to them.
  const [shown, setShown] = useState(1);
  const length = useRef(thread.length);
  useEffect(() => {
    if (length.current && thread.length > length.current) setShown((n) => n + thread.length - length.current);
    length.current = thread.length;
  }, [thread.length]);
  const from = Math.max(0, thread.length - shown);
  const visible = useMemo(() => thread.slice(from), [thread, from]);
  const ids = useMemo(() => new Set(thread.map((s) => s.id)), [thread]);

  // Only the shown sessions' logs: events of other sessions do not re-render the chat.
  const shownLogs = useSessionLogs(useShallow((st) => visible.map((s) => st.logs[s.id])));
  const logs = useMemo(() => Object.fromEntries(visible.map((s, i) => [s.id, shownLogs[i]])) as Record<string, SessionLog | undefined>, [visible, shownLogs]);
  const load = useSessionLogs((st) => st.load);
  const loadEarlier = useSessionLogs((st) => st.loadEarlier);
  useEffect(() => {
    for (const s of visible) if (!useSessionLogs.getState().logs[s.id]) {
      useSessionLogs.setState((st) => ({ logs: { ...st.logs, [s.id]: emptyLog() } }));
      void load(s.id);
    }
  }, [visible, load]);

  const presets = usePresets();
  const proposals = useProposalList().data;
  const today = useToday().data;
  const suggestions = useSuggestionList().data;
  const { cards: changed, activity } = useChanges(bot, ids);
  const byProposal = useMemo(() => new Map((proposals ?? []).map((p) => [p.id, p])), [proposals]);
  const byItem = useMemo(() => new Map((today?.items ?? []).flatMap((i) => [i, ...i.grouped]).map((i) => [i.id, i])), [today]);
  const bySuggestion = useMemo(() => new Map((suggestions ?? []).map((s) => [s.id, s])), [suggestions]);
  const cards = useMemo<ChatCard[]>(() => [
    ...(proposals ?? []).filter((p) => p.session_id && ids.has(p.session_id)).map((p): ChatCard => ({ kind: "proposal", id: p.id, session_id: p.session_id!, at: p.created_at, v: `${p.state}${p.updated_at}` })),
    ...(today?.items ?? []).flatMap((i) => [i, ...i.grouped]).filter((i) => READY.has(i.kind) && i.session_id && ids.has(i.session_id)).map((i): ChatCard => ({ kind: "ready", id: i.id, session_id: i.session_id!, at: i.at, v: "" })),
    ...changed,
  ], [proposals, today, changed, ids]);

  // A session shows once its newest events are loaded.
  const ready = visible.filter((s) => logs[s.id]?.loaded);
  const sessions = useMemo<ChatSession[]>(() => ready.map((s) => {
    const log = logs[s.id]!;
    let rows = folded.get(log);
    if (!rows) { rows = foldTrace(buildRows(log), s.launch_spec.text, presets); folded.set(log, rows); }
    return {
      id: s.id, started_at: s.started_at, profile: s.launch_spec.profile, folder: s.launch_spec.target.kind === "temp" ? null : s.project_id, reason: s.launch_spec.bot?.reason ?? null, note: s.launch_spec.bot?.note ?? null,
      running: isRunning(s), rows, firstTs: log.events[0]?.ts ?? null, partial: !!log.earlier, changes: s.status !== "closed",
    };
  }), [ready.map((s) => `${s.id}:${s.status}`).join(), shownLogs, presets]); // eslint-disable-line react-hooks/exhaustive-deps
  const first = ready[0] ? thread.indexOf(ready[0]) : 0;
  // Hook and Telegram messages waiting for the turn to end: grey rows after the chat, each with Cancel.
  const queued = useQuery({ queryKey: ["bot-queued", bot.id], queryFn: () => api.get<QueuedMessage[]>(`/api/bots/${bot.id}/queued`), retry: false }).data;
  const items = useMemo(() => [
    ...chatRows(sessions, cards, first),
    ...(queued ?? []).map((m): ChatItem => ({ t: "queued", key: `queued/${m.n}`, v: "", n: m.n, at: m.at, source: m.source, text: m.text })),
  ], [sessions, cards, first, queued]);
  const step = useMemo(() => { const w = items.findLast((i) => i.t === "working"); return w?.t === "working" ? w.step : null; }, [items]);
  useEffect(() => { onStep?.(step); }, [step, onStep]);
  const more = from > 0 || !!(ready[0] && logs[ready[0].id]?.earlier);
  const failedLog = visible.map((s) => logs[s.id]).find((l) => l?.error);

  // The chat shows the current session as Agents does: no notification about it, and seen after 1.5 s of use.
  const focused = useWindowFocused();
  const shownCurrent = current && logs[current.id]?.loaded ? current : null;
  const shownId = shownCurrent?.id ?? null;
  useEffect(() => { sendPresence(view, focused, shownId ? [shownId] : []); }, [view, focused, shownId]);
  useEffect(() => () => sendPresence(view, false, []), [view]);
  useMarkSeen(shownCurrent, focused);

  // The open question of the current session: typed text answers it, unless it asks for a permission.
  const ask = useMemo(() => {
    for (let i = items.length - 1; i >= 0; i--) {
      const it = items[i];
      if (it.t === "ask" && it.sessionId === current?.id && !it.row.answers) return { request_id: it.row.request_id, count: it.row.questions.length, permission: it.row.questions.some((q) => q.permission) };
    }
    return null;
  }, [items, current?.id]);
  // Continue only on the last turn of the current session, when it failed or was interrupted.
  const continueKey = useMemo(() => {
    if (!current || (current.status !== "error" && current.status !== "interrupted")) return null;
    const last = items.findLast((i) => i.t === "ended" || i.t === "user" || i.t === "from" || i.t === "answer" || i.t === "routine");
    return last?.t === "ended" && last.sessionId === current.id ? last.key : null;
  }, [items, current]);

  const scrollRef = useRef<HTMLDivElement>(null);
  const atBottom = useRef(true);
  const v = useVirtualizer({ count: items.length, getItemKey: (i) => items[i].key, getScrollElement: () => scrollRef.current, estimateSize: () => 56, overscan: 6, useFlushSync: false });
  const total = v.getTotalSize();
  useLayoutEffect(() => {
    if (atBottom.current && items.length) v.scrollToIndex(items.length - 1, { align: "end" });
  }, [items.length, total, v]);
  // Loading above keeps the row that was on top in view: once rows were added before it, scroll back to it.
  const anchor = useRef<{ key: string; index: number } | null>(null);
  useLayoutEffect(() => {
    const a = anchor.current;
    if (!a) return;
    const i = items.findIndex((it) => it.key === a.key);
    if (i < 0) anchor.current = null;
    else if (i !== a.index) { v.scrollToIndex(i, { align: "start" }); anchor.current = null; }
  }, [items, v]);
  const [loadingMore, setLoadingMore] = useState(false);
  const loadMore = () => {
    if (loadingMore || !more || ready.length < visible.length) return;
    const top = scrollRef.current?.scrollTop ?? 0;
    const shownTop = v.getVirtualItems().find((it) => it.end > top);
    anchor.current = shownTop ? { key: String(shownTop.key), index: shownTop.index } : null;
    atBottom.current = false;
    const first = ready[0];
    if (first && logs[first.id]?.earlier) {
      setLoadingMore(true);
      void loadEarlier(first.id).catch(toastError).finally(() => setLoadingMore(false));
    } else setShown((n) => n + 1);
  };

  return (
    <div className="flex h-full min-h-0 min-w-0 flex-1 flex-col bg-paper" data-tour="bots.thread">
      {failedLog && <div role="alert" className="flex flex-wrap items-center gap-3 px-4 py-2 text-[12px] text-ink-3">
        <span className="min-w-0 flex-1 break-words">Could not load the thread: {failedLog.error}</span>
        <button className="btn btn-sm" onClick={() => visible.forEach((s) => void load(s.id))}>Retry</button>
      </div>}
      <div ref={scrollRef} className="lp-quiet-scroll min-h-0 flex-1 overflow-x-hidden overflow-y-auto px-3 [overflow-wrap:anywhere]"
        onScroll={(e) => {
          const el = e.currentTarget;
          atBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 60;
          if (atBottom.current) anchor.current = null;
          if (el.scrollTop < 150) loadMore();
        }}>
        <div className="mx-auto w-full max-w-text">
          {thread.length === 0 ? <div className="py-8">{intro}</div> : <>
            {more && <div className="flex justify-center pt-2"><button className="btn btn-sm" disabled={loadingMore} onClick={loadMore}>{loadingMore ? "Loading…" : "Load earlier messages"}</button></div>}
            {sessions.length === 0 && <div className="p-6 text-center text-[13px] text-ink-3">Loading…</div>}
            <div style={{ height: total, position: "relative" }}>
              {v.getVirtualItems().map((it) => {
                const item = items[it.index];
                const data = item.t === "card" ? item.kind === "proposal" ? byProposal.get(item.id) : item.kind === "ready" ? byItem.get(item.id) : activity.get(item.id) : undefined;
                const extra = item.t === "card" && item.kind === "ready" && data ? bySuggestion.get((data as TodayItem).record) : undefined;
                return (
                  <div key={it.key} data-index={it.index} ref={v.measureElement} className="absolute top-0 right-0 left-0" style={{ transform: `translateY(${it.start}px)` }}>
                    <ChatRow item={item} data={data} extra={extra} canContinue={item.key === continueKey} botId={bot.id} />
                  </div>
                );
              })}
            </div>
          </>}
          <div className="h-3" />
        </div>
      </div>
      <BotComposer bot={bot} current={current} rotation={rotation} needsProfile={needsProfile} openSettings={openSettings} ask={ask}
        newSession={newSession} onNewSession={onNewSession} onSent={() => { atBottom.current = true; }} />
    </div>
  );
}

// ---------------------------------------------------------------- rows

/** A hook or Telegram message waiting for the bot's turn to end, with Cancel. */
function QueuedRow({ item, botId }: { item: Extract<ChatItem, { t: "queued" }>; botId: string }) {
  const qc = useQueryClient();
  return <div className="py-1" data-tour="chat.queued"><div className="flex items-center gap-2 rounded-card border border-dashed border-line px-2.5 py-1.5 text-[13px] text-ink-3">
    <Icon name="clock" size={12} className="flex-none" />
    <span className="min-w-0 flex-1">
      <span className="line-clamp-2 break-words whitespace-pre-wrap">{item.text}</span>
      <span className="block text-[11px]">Waiting · from {item.source === "telegram" ? "Telegram" : "a hook"} · {ago(item.at)} · sent when the current turn ends</span>
    </span>
    <ActionButton name="bot.cancel-queued" params={{ id: botId, n: String(item.n) }} icon={null} onDone={() => qc.invalidateQueries({ queryKey: ["bot-queued", botId] })} />
  </div></div>;
}

const ChatRow = memo(function ChatRow({ item, data, extra, canContinue, botId }: { item: ChatItem; data: Proposal | TodayItem | Activity | Change | undefined; extra: Suggestion | undefined; canContinue: boolean; botId: string }) {
  switch (item.t) {
    case "divider":
      return <div className="pt-4 pb-2"><Separator>
        <span>Session {item.index} · started {when(item.at)}{item.reason ? ` because ${item.reason}` : ""}</span>
      </Separator></div>;
    case "user":
      return <div className="flex py-1.5"><div className="ml-auto max-w-[85%] rounded-card rounded-br-sm bg-sunk px-2.5 py-[7px] text-[13px] whitespace-pre-wrap">
        {item.text}
        {item.images.length > 0 && <div className="flex flex-wrap gap-2">{item.images.map((image, i) => { const src = image.data_url.startsWith("/api/") ? daemonAsset(image.data_url) : image.data_url; return <a key={i} href={src} download={image.name} title={`Save ${image.name}`}><img src={src} alt={image.name} className="mt-2 max-h-40 max-w-full rounded-control border border-line" /></a>; })}</div>}
        {item.queued && <span className="ml-2 text-[11px] text-ink-3">queued: read after the current step</span>}
        {item.instructions && <span className="mt-1 flex items-center gap-1 text-[11px] text-ink-3" title="BOT.md changed since this session last read it: the new BOT.md went with this message"><Icon name="file" size={12} />With your updated BOT.md</span>}
      </div></div>;
    case "from":
      return <FromRow item={item} />;
    case "routine":
      return <div className="py-1.5"><Separator><span className="flex min-w-0 items-center gap-1.5" title="A routine started this run"><Icon name="refresh" size={12} className="flex-none" /><span className="min-w-0">Routine “<RoutineName title={item.title} />” · {item.cause}</span></span></Separator></div>;
    case "quiet":
      return <div className="flex items-center gap-1.5 py-1 text-[12px] text-ink-3" title="The run had nothing to report: no notification"><Icon name="refresh" size={12} /><span className="truncate"><RoutineName title={item.title} /> · {item.cause}{item.at ? ` · ${when(item.at)}` : ""} · no update</span></div>;
    case "answer":
      return <div className="py-1.5 text-[13px]"><AssistantRow text={item.text} streaming={item.streaming} /></div>;
    case "steps":
      return <StepsRow item={item} />;
    case "ask":
      return <div className="py-1.5"><AskLive item={item} /></div>;
    case "ended":
      return <EndedRow item={item} canContinue={canContinue} />;
    case "card":
      if (item.kind === "changed") {
        // `act…` lines are direct changes (the activity list), `kb…` lines knowledge-base changes.
        const a = item.id.startsWith("act") ? data as Activity | undefined : undefined;
        const c = item.id.startsWith("kb") ? data as Change | undefined : undefined;
        return <div className="flex min-h-7 items-center gap-2 py-1 text-[12px] text-ink-2" data-tour="chat.changed">
          <span className="rounded bg-sunk px-1.5 text-[11px] font-semibold text-ink-3">Changed</span>
          <span className="min-w-0 flex-1 truncate" title={a ? `${(ACTIONS as Record<string, { help: string }>)[a.action]?.help ?? ""}: ${item.text}` : item.text ?? undefined}>
            {a ? <ActivityText a={a} /> : c ? <KbText c={c} /> : item.text}
          </span>
        </div>;
      }
      if (!data) return null;
      return <div className="grid min-w-0 grid-cols-[minmax(0,1fr)] py-1.5">{item.kind === "proposal" ? <ProposalCard p={data as Proposal} /> : <ItemCard item={data as TodayItem} proposal={undefined} suggestion={extra} />}</div>;
    case "changes":
      return <ChangesRow sessionId={item.sessionId} />;
    case "working":
      return <WorkingRow item={item} />;
    case "queued":
      return <QueuedRow item={item} botId={botId} />;
  }
}, (a, b) => a.item.key === b.item.key && a.item.v === b.item.v && a.data === b.data && a.extra === b.extra && a.canContinue === b.canContinue && a.botId === b.botId);

/** A message from another bot or agent (Home stamped its sender): on the left under the sender's mark and name, never
 * as your bubble. */
function FromRow({ item }: { item: Extract<ChatItem, { t: "from" }> }) {
  const f = item.from;
  const line = senderLine(f);
  return <div className="flex items-start gap-2 py-1.5" data-tour="chat.from">
    {f.kind === "bot" ? <BotAvatar name={f.name} size={24} /> : <span className="grid size-6 flex-none place-items-center rounded-card bg-sunk text-ink-3" aria-hidden><Icon name="agents" size={12} /></span>}
    <div className="min-w-0 max-w-[85%] rounded-card rounded-tl-sm border border-line px-2.5 py-[7px] text-[13px]">
      <div className="mb-0.5 text-[11px] text-ink-3" title="Launchpad stamped the sender: the bot read this message as information, not as your instructions">
        {f.kind === "bot"
          ? <Link to="/bots/$id" params={{ id: f.id }} className={NAME_LINK}>{line}</Link>
          : <Link to="/agents" search={{ session: f.id }} className={NAME_LINK}>{line}</Link>}
      </div>
      <div className="break-words whitespace-pre-wrap">{item.text}</div>
      {item.images.length > 0 && <div className="flex flex-wrap gap-2">{item.images.map((image, i) => { const src = image.data_url.startsWith("/api/") ? daemonAsset(image.data_url) : image.data_url; return <a key={i} href={src} download={image.name} title={`Save ${image.name}`}><img src={src} alt={image.name} className="mt-2 max-h-40 max-w-full rounded-control border border-line" /></a>; })}</div>}
      {item.queued && <span className="text-[11px] text-ink-3">queued: read after the current step</span>}
      {item.instructions && <span className="mt-1 flex items-center gap-1 text-[11px] text-ink-3" title="BOT.md changed since this session last read it: the new BOT.md went with this message"><Icon name="file" size={12} />With your updated BOT.md</span>}
    </div>
  </div>;
}

/** A name in a chat line that opens its page: the same text, underlined on hover. */
const NAME_LINK = "text-inherit no-underline hover:underline";

const kbVerb = (c: Change) => c.new_sha256 ? (c.sha256 ? "Edited" : "Created") : "Removed";
const kbStats = (c: Change) => c.added != null ? ` · +${c.added} −${c.removed ?? 0}` : "";
const kbLine = (c: Change) => `${kbVerb(c)} ${c.path}${kbStats(c)}`;

/** A knowledge-base change: the note's path opens it in Knowledge (a removed note has nothing to open). */
function KbText({ c }: { c: Change }) {
  return <>{kbVerb(c)} {c.new_sha256 ? <Link to="/knowledge" search={{ note: c.path }} className={NAME_LINK}>{c.path}</Link> : c.path}{kbStats(c)}</>;
}

/** A routine run's title: opens the routine in Schedules when a routine still has that title. */
function RoutineName({ title }: { title: string }) {
  const id = useCrons().data?.find((c) => c.title === title)?.id;
  return id ? <Link to="/schedules" search={{ routine: id }} className={NAME_LINK}>{title}</Link> : <>{title}</>;
}

/** A direct change: the record's name opens its page. */
function ActivityText({ a }: { a: Activity }) {
  const p = activityParts(a);
  const link = activityLink(a);
  return <>{p.before}{p.target && (link ? <Link {...link} className={NAME_LINK}>{p.target}</Link> : p.target)}{p.after}</>;
}

function StepsRow({ item }: { item: Extract<ChatItem, { t: "steps" }> }) {
  const [open, setOpen] = useState(false);
  return <div className="py-1">
    <div className="flex items-center gap-2 text-[12px] text-ink-3">
      <button className="chat-tap flex items-center gap-1 hover:text-ink" aria-expanded={open} title="The work behind this answer, one line per step" onClick={() => setOpen(!open)}>
        <Icon name="chev" size={12} className={`transition-transform ${open ? "" : "-rotate-90"}`} />{stepsLabel(item.count, item.durationMs)}
      </button>
      <Link to="/agents" search={{ session: item.sessionId }} className="chat-tap text-teal-ink no-underline hover:underline" title="The full trace in Agents">Open trace</Link>
    </div>
    {open && <ol className="m-0 mt-1 grid gap-0.5 border-l-2 border-line-2 pl-3 text-[12px] text-ink-3">{item.steps.map((s, i) => <li key={i} className="list-none truncate font-mono text-[11px]" title={s}>{s}</li>)}</ol>}
  </div>;
}

function AskLive({ item }: { item: Extract<ChatItem, { t: "ask" }> }) {
  const live = useSession(item.sessionId)?.live ?? false;
  return <AskCard row={item.row} sessionId={item.sessionId} live={live} />;
}

function EndedRow({ item, canContinue }: { item: Extract<ChatItem, { t: "ended" }>; canContinue: boolean }) {
  const [busy, setBusy] = useState(false);
  return <div className="flex flex-wrap items-center gap-2 py-1.5 text-[12px] text-gh-closed">
    <Icon name="xcircle" size={12} />
    <span className="min-w-0 flex-1 break-words">{item.error ? `The turn failed: ${item.error}` : "The turn was interrupted."}</span>
    {canContinue && <button className="btn btn-sm" disabled={busy} title="Continue the unfinished task" onClick={() => { setBusy(true); void runAction("session.resume", { id: item.sessionId }, { continue_turn: true }).catch(toastError).finally(() => setBusy(false)); }}>Continue</button>}
  </div>;
}

/** What the session changed in its worktree: the branch, and Fix merge when a merge stopped, else Merge when it is
 * safe, else Review changes. */
function ChangesRow({ sessionId }: { sessionId: string }) {
  const s = useSession(sessionId);
  const { safe, work, stopped } = useBranch(sessionId, s);
  if (!safe && !work && !stopped) return null;
  return <div className="py-1.5"><div className={`grid gap-2 rounded-card border bg-paper p-3 ${stopped ? "border-attn-line" : "border-line"}`}>
    <div className="text-[13px] font-semibold">Changes</div>
    <BranchActions sessionId={sessionId} s={s} />
  </div></div>;
}

function WorkingRow({ item }: { item: Extract<ChatItem, { t: "working" }> }) {
  const s = useSession(item.sessionId);
  const [busy, setBusy] = useState(false);
  const [, tick] = useState(0);
  useEffect(() => { const t = setInterval(() => tick((n) => n + 1), 5_000); return () => clearInterval(t); }, []);
  if (!s || !isRunning(s)) return null;
  const label = s.status === "starting" ? "Starting" : s.status === "background" ? "Background work" : "Working";
  return <div className="flex items-center gap-2 py-1.5 text-[12px] text-work" role="status">
    <span className="lp-spin" />
    <span className="min-w-0 flex-1 truncate">{label}{item.step ? ` · ${item.step}` : ""} · {age(s.status_since)}</span>
    <button className="btn btn-sm" disabled={busy} title="Stop: interrupt the bot" onClick={() => { setBusy(true); void runAction("session.stop", { id: s.id }).catch(toastError).finally(() => setBusy(false)); }}><Icon name="stop" size={12} />Stop</button>
  </div>;
}

// ---------------------------------------------------------------- the message box

function BotComposer({ bot, current, rotation, needsProfile, openSettings, ask, newSession, onNewSession, onSent }: {
  bot: Bot; current: Session | null; rotation: string | null; needsProfile: boolean;
  openSettings: () => void;
  ask: { request_id: string; count: number; permission: boolean } | null;
  newSession: NewSessionRequest | null;
  onNewSession?: (r: NewSessionRequest | null) => void;
  onSent: () => void;
}) {
  const qc = useQueryClient();
  const harness = useHarnessInfo(current?.harness ?? null);
  const acceptsImages = harness ? harness.capabilities.images : true;
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  // ⋯ → Start a new session: the next message starts a new session of the thread (the page keeps the request).
  const fresh = !!newSession;
  const ref = useRef<HTMLTextAreaElement>(null);
  const fileRef = useRef<HTMLInputElement>(null);
  const attachments = useMessageAttachments(acceptsImages);
  const { files, reading } = attachments;
  // Context and presets for the next message, as in an agent's message box (never with an answer to a question).
  const [storedContext, setContext] = useState(EMPTY_CONTEXT);
  const [selectedPresets, setPresets] = useState<string[]>([]);
  const botPresets = usePresets("bots");
  // Grows with the text up to 10 lines, then scrolls.
  useLayoutEffect(() => { const el = ref.current; if (!el) return; el.style.height = "auto"; el.style.height = `${Math.min(el.scrollHeight, 220)}px`; }, [text]);
  const answering = !!ask && !fresh && !!current?.open_question && !!current.live;
  // Typed text never grants a permission; a harness that cannot take messages while it works waits for the turn end.
  const busyTurn = !fresh && !!current && isRunning(current) && !current.open_question && !!harness && !harness.capabilities.messages_while_running;
  const blocked = (answering && ask!.permission) || busyTurn;
  const context = answering ? EMPTY_CONTEXT : storedContext;
  const presets = answering ? [] : selectedPresets.filter((id) => botPresets.some((p) => p.id === id));
  const withContext = hasMessageContext(context);
  const send = async (typed = text) => {
    const t = answering ? typed.trim() : withPresets(typed, presets, botPresets);
    if ((!t && !files.length && !withContext) || busy || reading || blocked) return;
    setBusy(true);
    try {
      if (answering && current) {
        await runAction("session.answer", { id: current.id }, { request_id: ask!.request_id, answers: Array(ask!.count).fill(t || "See the attachments in my next message.") });
        if (files.length) await runAction("bot.message", { id: bot.id }, { text: "", attachments: files });
      } else {
        await runAction("bot.message", { id: bot.id }, { text: t, attachments: files, ...(fresh ? { new_session: true, reason: newSession!.reason } : {}), ...(withContext ? { context } : {}) });
        onNewSession?.(null);
        setContext(EMPTY_CONTEXT);
        setPresets([]);
      }
      setText("");
      attachments.clear();
      onSent();
      await qc.invalidateQueries({ queryKey: ["sessions"] });
    } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  if (needsProfile)
    return <div className="border-t border-line-2 p-2.5"><div className="flex items-center gap-2 rounded-control bg-attn-soft px-3 py-2 text-[13px] text-attn">This bot has no profile.<button className="btn btn-sm ml-auto" onClick={openSettings}>Choose a profile</button></div></div>;
  return (
    <div className="border-t border-line-2 px-2.5 pt-2 pb-2.5" data-tour="bots.reply">
      <div className="mx-auto w-full max-w-text">
        {rotation && current && <div className="mb-1.5 text-[12px] text-ink-3">{rotation}: your next message starts a new session of {bot.settings.name}'s thread, with a reference to the previous one.</div>}
        {fresh && !rotation && <div className="mb-1.5 flex items-center gap-1.5 text-[12px] text-ink-3" data-tour="bots.new-session"><Icon name="plus" size={12} className="flex-none" /><span className="min-w-0 flex-1 truncate">Your next message starts a new session{newSession!.reason ? ` · “${newSession!.reason}”` : ""}</span><button className="chat-tap flex-none text-teal-ink hover:underline" onClick={() => onNewSession?.(null)}>Cancel</button></div>}
        {!rotation && !fresh && instructionsUpdated(bot, current) && <div className="mb-1.5 flex items-center gap-1.5 text-[12px] text-ink-3" data-tour="bots.instructions-updated" title={`BOT.md changed since ${bot.settings.name}'s current session read it: the new BOT.md goes with your next message, in the same session`}><Icon name="file" size={12} />Instructions updated · applies to your next message</div>}
        <input ref={fileRef} type="file" multiple hidden onChange={(e) => { void attachments.add(Array.from(e.target.files ?? [])); e.target.value = ""; }} />
        <AttachmentList files={files} disabled={busy || reading} onRemove={attachments.remove} />
        {!answering && <PresetChips surface="bots" selected={presets} onChange={setPresets} disabled={busy} className="mb-1.5" />}
        {!answering && <MessageContextChips className="mb-1.5" value={context} onChange={setContext} disabled={busy || reading} />}
        {!text && !files.length && !withContext && !presets.length && !answering && !blocked && !busy && <QuickChips starters={bot.settings.starters ?? []} onSend={(t) => void send(t)} onFill={(t) => { setText(t); requestAnimationFrame(() => { const el = ref.current; if (el) { el.focus(); el.setSelectionRange(t.length, t.length); } }); }} />}
        <div className="session-message-input flex items-end gap-2 rounded-control border border-line px-2.5 py-[7px] focus-within:border-teal focus-within:shadow-[0_0_0_3px_var(--color-teal-soft)]">
          <textarea ref={ref} rows={1} value={text} disabled={busy || blocked} aria-label={`Message ${bot.settings.name}`}
            placeholder={busyTurn ? `${bot.settings.name} is working: Stop it or wait for the turn to end.` : blocked ? "Choose a decision on the approval card." : answering ? `Answer or message ${bot.settings.name}…` : `Message ${bot.settings.name}…`}
            onChange={(e) => setText(e.target.value)}
            onPaste={(e) => { if (!acceptsImages || busy) return; const pasted = pastedImages(e); if (pasted) void attachments.add(pasted); }}
            onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); void send(); } }}
            className="min-h-6 min-w-0 flex-1 resize-none bg-transparent text-[13px] leading-5 outline-none" />
          {!answering && <MessageContextButton sessionId={current?.id ?? null} value={context} onChange={setContext} presets={{ presets: botPresets, selected: presets, onChange: setPresets }} disabled={busy || reading || blocked} />}
          <button className="iconbtn" title={`Attach files${acceptsImages ? " or images" : ""}. ${ATTACH_LIMITS}`} aria-label="Attach files" disabled={busy || reading || blocked} onClick={() => fileRef.current?.click()}><Icon name="clip" size={14} /></button>
          <button className="iconbtn border-line! bg-paper" title="Send" aria-label="Send" disabled={busy || reading || blocked || (!text.trim() && !files.length && !withContext && !presets.length)} onClick={() => void send()}><Icon name="send" size={14} /></button>
        </div>
      </div>
    </div>
  );
}

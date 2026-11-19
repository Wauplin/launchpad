// The chat of a bot: every bot page shows it (Ask Launchpad has no code of its own). It draws the sessions of the
// bot's thread from their events (chatRows.ts): your messages, the bot's answers, one quiet steps line per turn
// (with receipts for its changes), its questions with their options as chips, and what each turn wrote: proposals
// as compact lines that become receipts once handled, and ready clicks. The trace stays in Agents (the page's ⋯).
// Newest at the bottom; older events and sessions load as you scroll up, tail-first. It fills its parent, so it
// works as a page, a panel and the phone's full screen. It shows one thread of the bot (its parallel conversations,
// BotThreads.tsx); the one message box sends to that thread (a new one for New thread), or answers its open question
// or a question report ("Answering: …", with Cancel to send a normal message instead).
// A thread that other bots joined (a conversation of several bots, `useConversation`) shows every bot's thread merged
// into one timeline (`mergeConversation`), with a status line per bot; the box sends to the main bot, or to the bots
// its leading @-mentions name, and answers a question of whichever bot asked it.

import { useProfiles, useProfileModels } from "@/lib/profiles";
import { useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { createContext, Fragment, memo, useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import type { Bot } from "@/gen/Bot";
import type { BotSend } from "@/gen/BotSend";
import type { Change } from "@/gen/Change";
import type { Proposal } from "@/gen/Proposal";
import type { QueuedMessage } from "@/gen/QueuedMessage";
import { ActionButton, ActionMenuItem } from "@/components/ActionButton";
import type { Session } from "@/gen/Session";
import type { Suggestion } from "@/gen/Suggestion";
import { BotAvatar, OwnerAvatar, SourceAvatar, botInk } from "@/components/BotAvatar";
import { useHarnessInfo } from "@/components/HarnessMark";
import { Icon } from "@/components/Icon";
import { ImageThumb } from "@/components/ImageViewer";
import { toastError } from "@/components/Toast";
import { AskCard } from "@/features/agents/AskCard";
import { AssistantRow, Separator } from "@/features/agents/EventRow";
import { ATTACH_LIMITS, AttachmentList, useMessageAttachments } from "@/features/agents/MessageAttachments";
import { PresetChips, usePresets } from "@/features/agents/MessagePresets";
import { EMPTY_CONTEXT, hasMessageContext, MessageContextButton, MessageContextChips } from "@/features/agents/MessageContext";
import { withPresets } from "@/lib/presets";
import { ApiError } from "@/lib/api";
import { abandonOperation } from "@/lib/operations";
import { senderParts } from "@/lib/senders";
import { ChatMessage, MESSAGE_INDENT, SenderTag } from "./Message";
import { instructionsUpdated, sentHere, showAccepted } from "@/lib/bots";
import { useMarkSeen, useWindowFocused } from "@/features/agents/useSeen";
import { buildRows } from "@/features/agents/rows";
import { foldTrace } from "@/features/agents/traceRows";
import { ProposalLine, useProposalList } from "@/features/proposals/ProposalCard";
import { useSuggestionList } from "@/features/suggestions/data";
import { useToday, type TodayItem } from "@/features/today/data";
import { BranchActions, githubRef, ItemCard, useBranch } from "@/features/today/TodayCards";
import { githubItemUrl, suggestionItemUrl } from "@/lib/githubLinks";
import { openExternal } from "@/lib/platform";
import { LOOK } from "@/features/today/TodayRows";
import { MenuItem } from "@/components/Popover";
import { RowActions } from "@/components/ui";
import { usePhone } from "@/lib/phone";
import { runAction } from "@/lib/actions";
import { ACTIONS } from "@/gen/actions";
import type { Activity } from "@/gen/Activity";
import type { FolioActivity } from "@/gen/FolioActivity";
import { folioReceipt, useFolioActivity } from "@/lib/folios";
import { activityLine, activityLink, activityParts, activityReceipt, useActivity } from "./activity";
import { api } from "@/lib/api";
import { attachmentSrc, isImageAttachment, pastedImages } from "@/lib/attachments";
import { age, ago, when } from "@/lib/format";
import { chatDraftKey, dropSentChatDraft, useKeptDraft } from "@/lib/keptDraft";
import { refusedFor } from "@/lib/maintenance";
import { NEW_THREAD } from "./BotThreads";
import { useCrons, useSession, useHarnessDefaults } from "@/lib/queries";
import { emptyLog, useSessionLogs, type SessionLog } from "@/lib/sessionStore";
import { isRunning, isSleeping, sleepUntil } from "@/lib/sessionStatus";
import { sendPresence } from "@/lib/ws";
import { QuickChips } from "./QuickChips";
import { chatRows, pendingRows, sleepKey, stepsLabel, type ChatCard, type ChatItem, type ChatSession } from "./chatRows";
import { useConversation, type ConversationInfo } from "./useConversation";
import { completeMention, conversationStatus, mentionChoices, mentionQuery, mergeConversation, routeMentions } from "@/lib/conversations";
import { useSessions } from "@/lib/queries";
import { isUnseen } from "@/lib/sessionStatus";

/** A record behind a change receipt: a knowledge-base change, a change of the bot's own files, or a direct change. */
type ChangeRecord = { kind: "kb"; c: Change } | { kind: "bot"; c: Change; botId: string } | { kind: "act"; a: Activity } | { kind: "folio"; f: FolioActivity };

/** What the message box answers instead of sending a message: the open question, or a question report. */
type Reply = { kind: "ask"; request_id: string; count: number; permission: boolean; question: string; sessionId: string; bot: string | null } | { kind: "report"; id: string; title: string };

/** Lets a question report's Reply (in a row) aim the message box at it. */
const ReplyContext = createContext<(p: Proposal) => void>(() => {});

/** Today kinds that are a ready click in a bot's chat: Send, Post, Review & publish. */
const READY = new Set<TodayItem["kind"]>(["outbox", "suggestion", "local_pr"]);


// Display rows per log object: a session whose events did not change keeps its rows.
const folded = new WeakMap<SessionLog, ReturnType<typeof foldTrace>>();

/** What the thread's sessions changed: knowledge-base notes, the bot's own files (its memory, BOT.md, routines) and
 * Launchpad records they changed directly at admin access (the activity list). Receipts on the steps line of their
 * turn, with the full line when the steps are open. */
function useChanges(bot: Bot, ids: Set<string>): { cards: ChatCard[]; records: Map<string, ChangeRecord> } {
  const ws = bot.workspace_id;
  const kb = useQuery({ queryKey: ["knowledge", ws, "changes"], queryFn: () => api.get<{ changes: Change[] }>(`/api/knowledge/${ws}/changes`) }).data?.changes;
  const own = useQuery({ queryKey: ["bot-history", bot.id, bot.version], queryFn: () => api.get<Change[]>(`/api/bots/${bot.id}/history`) }).data;
  const done = useActivity(ws).data;
  const folios = useFolioActivity(ws).data;
  return useMemo(() => {
    const of = (c: Change) => c.author?.session_id ?? c.session_id;
    const mine = (done ?? []).filter((a) => ids.has(a.session_id));
    const files = (own ?? []).filter((c) => ids.has(of(c) ?? ""));
    const notes = (kb ?? []).filter((c) => ids.has(of(c) ?? ""));
    const written = (folios ?? []).filter((f) => f.session_id && ids.has(f.session_id));
    return {
      cards: [
        ...notes.map((c): ChatCard => ({ kind: "changed", id: `kb${c.id}`, session_id: of(c)!, at: c.at, v: "", text: kbLine(c), receipt: `${c.new_sha256 ? "noted" : "removed"} ${c.path} in the knowledge base` })),
        ...files.map((c): ChatCard => ({ kind: "changed", id: `bot${c.id}`, session_id: of(c)!, at: c.at, v: "", text: kbLine(c), receipt: botFileReceipt(c) })),
        ...mine.map((a): ChatCard => ({ kind: "changed", id: `act${a.id}`, session_id: a.session_id, at: a.created_at, v: "", text: activityLine(a), receipt: activityReceipt(a) })),
        ...written.map((f): ChatCard => ({ kind: "changed", id: `folio${f.folio_id}:${f.rev}`, session_id: f.session_id!, at: f.at, v: "", text: `${folioReceipt(f)} (${f.paths.join(", ")})`, receipt: folioReceipt(f) })),
      ],
      records: new Map<string, ChangeRecord>([
        ...notes.map((c): [string, ChangeRecord] => [`kb${c.id}`, { kind: "kb", c }]),
        ...files.map((c): [string, ChangeRecord] => [`bot${c.id}`, { kind: "bot", c, botId: bot.id }]),
        ...mine.map((a): [string, ChangeRecord] => [`act${a.id}`, { kind: "act", a }]),
        ...written.map((f): [string, ChangeRecord] => [`folio${f.folio_id}:${f.rev}`, { kind: "folio", f }]),
      ]),
    };
  }, [kb, own, done, folios, ids, bot.id]);
}

/** A change of the bot's own files, as a receipt: "noted in its memory", "changed its BOT.md". */
function botFileReceipt(c: Change): string {
  if (c.path.startsWith("memory/")) return c.new_sha256 ? "noted in its memory" : "removed a note from its memory";
  const what = c.path === "BOT.md" ? "its BOT.md" : c.path.startsWith("routines/") ? `its routine ${c.path.slice(9).replace(/\.toml$/, "")}` : c.path;
  return `${c.new_sha256 ? (c.sha256 ? "changed" : "added") : "deleted"} ${what}`;
}

export function BotChat({ bot, threadId, thread, rotation, needsProfile, openSettings, intro, view = "bot", onStep, onThread, conversationBar = true }: {
  bot: Bot;
  /** Which view shows the chat (a bot page, the Ask panel), for the sessions this window reports on screen. */
  view?: string;
  /** The thread shown: its ID, or `new` (New thread: the next message starts one). */
  threadId: string;
  /** The thread's session (none yet: empty; older threads: their newest). */
  thread: Session[];
  /** Your message went to another thread than the one shown (New thread's first message, or a thread that continues
   * in a new one): the page shows that thread. */
  onThread?: (thread: string) => void;
  /** Why the next message continues in a new thread (archived, or the profile or folder changed…), or null. */
  rotation: string | null;
  needsProfile: boolean;
  openSettings: () => void;
  /** Shown while the thread is empty. */
  intro: React.ReactNode;
  /** The step the current turn works on (its last step title), for the page's status line. */
  onStep?: (step: string | null) => void;
  /** The bar over a conversation of several bots (its bots and their states); a page whose header names them hides it. */
  conversationBar?: boolean;
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
  // The conversation of the thread: other bots that joined it show in the same timeline.
  const conversation = useConversation(bot, threadId === NEW_THREAD ? null : threadId);
  const others = useMemo(() => (conversation?.participants ?? []).filter((p) => p.bot.id !== bot.id), [conversation]);
  const multi = others.length > 0;
  const otherSessions = useMemo(() => others.flatMap((p) => p.sessions), [others]);
  const ids = useMemo(() => new Set([...thread, ...otherSessions].map((s) => s.id)), [thread, otherSessions]);
  // Every session whose events load: the shown thread's, and every session of the other bots' threads.
  const loaded = useMemo(() => [...visible, ...otherSessions], [visible, otherSessions]);

  // Only the shown sessions' logs: events of other sessions do not re-render the chat.
  const shownLogs = useSessionLogs(useShallow((st) => loaded.map((s) => st.logs[s.id])));
  const logs = useMemo(() => Object.fromEntries(loaded.map((s, i) => [s.id, shownLogs[i]])) as Record<string, SessionLog | undefined>, [loaded, shownLogs]);
  const load = useSessionLogs((st) => st.load);
  const loadEarlier = useSessionLogs((st) => st.loadEarlier);
  useEffect(() => {
    for (const s of loaded) if (!useSessionLogs.getState().logs[s.id]) {
      useSessionLogs.setState((st) => ({ logs: { ...st.logs, [s.id]: emptyLog() } }));
      void load(s.id);
    }
  }, [loaded, load]);

  const presets = usePresets();
  const proposals = useProposalList().data;
  const today = useToday().data;
  const suggestions = useSuggestionList().data;
  const { cards: changed, records } = useChanges(bot, ids);
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
  const toChat = (s: Session): ChatSession => {
    const log = logs[s.id]!;
    let rows = folded.get(log);
    if (!rows) { rows = foldTrace(buildRows(log), s.launch_spec.text, presets); folded.set(log, rows); }
    return {
      id: s.id, started_at: s.started_at, profile: s.launch_spec.profile, folder: s.launch_spec.target.kind === "temp" ? null : s.project_id, reason: s.launch_spec.bot?.reason ?? null, note: s.launch_spec.bot?.note ?? null, previous: s.launch_spec.bot?.previous?.title ?? null,
      running: isRunning(s), rows, firstTs: log.events[0]?.ts ?? null, partial: !!log.earlier, changes: s.status !== "closed", sleep: isSleeping(s) ? s.sleep : null,
    };
  };
  // A sleep starts, changes or ends without a new status (Done stays Done): it is part of the key.
  const sessionKey = (list: Session[]) => list.map((s) => `${s.id}:${s.status}:${s.open_question ? "q" : ""}:${s.sleep ? sleepKey(s.sleep) : ""}`).join();
  const sessions = useMemo<ChatSession[]>(() => ready.map(toChat), [sessionKey(ready), shownLogs, presets]); // eslint-disable-line react-hooks/exhaustive-deps
  const otherReady = otherSessions.filter((s) => logs[s.id]?.loaded);
  const otherChats = useMemo(() => others.map((p) => ({ p, sessions: p.sessions.filter((s) => logs[s.id]?.loaded).map(toChat) })), [others, sessionKey(otherReady), shownLogs, presets]); // eslint-disable-line react-hooks/exhaustive-deps
  const first = ready[0] ? thread.indexOf(ready[0]) : 0;
  // Hook and Telegram messages waiting for the turn to end: grey rows after the chat, each with Cancel.
  const queued = useQuery({ queryKey: ["bot-queued", bot.id], queryFn: () => api.get<QueuedMessage[]>(`/api/bots/${bot.id}/queued`), retry: false }).data;
  // Your messages Home accepted that no loaded session shows yet: their bubbles, with how far each got.
  const sends = useQuery({ queryKey: ["bot-sends", bot.id], queryFn: () => api.get<BotSend[]>(`/api/bots/${bot.id}/sends`), retry: false }).data;
  // The same for the other bots of the conversation.
  const otherQueued = useQueries({ queries: others.map((p) => ({ queryKey: ["bot-queued", p.bot.id], queryFn: () => api.get<QueuedMessage[]>(`/api/bots/${p.bot.id}/queued`), retry: false })) }).map((q) => q.data);
  const otherSends = useQueries({ queries: others.map((p) => ({ queryKey: ["bot-sends", p.bot.id], queryFn: () => api.get<BotSend[]>(`/api/bots/${p.bot.id}/sends`), retry: false })) }).map((q) => q.data);
  // Agents the bots of the conversation launched: a row each, at its start.
  const allSessions = useSessions().data;
  const launched = useMemo(() => multi ? (allSessions ?? []).filter((s) => s.launch_spec.from?.session && ids.has(s.launch_spec.from.session)) : [], [multi, allSessions, ids]);
  const items = useMemo((): ChatItem[] => {
    const own = (list: ChatSession[], firstIndex: number, botSends: BotSend[] | undefined, botQueued: QueuedMessage[] | undefined, thread: string): ChatItem[] => [
      ...chatRows(list, cards, firstIndex),
      ...pendingRows(botSends ?? [], list, thread),
      ...(botQueued ?? []).filter((m) => m.thread === thread).map((m): ChatItem => ({ t: "queued", key: `queued/${m.n}`, v: `${m.slot}`, n: m.n, at: m.at, source: m.source, text: m.text, slot: m.slot })),
    ];
    const mine = own(sessions, first, sends, queued, threadId);
    if (!multi || !conversation) return mine;
    const launches = (botId: string, list: Session[]): ChatItem[] => launched.filter((l) => list.some((s) => s.id === l.launch_spec.from?.session) && l.launch_spec.from?.id === botId)
      .map((l) => ({ t: "launch", key: `launch/${l.id}`, v: l.status, at: l.started_at, sessionId: l.id, title: l.title, status: l.status }));
    return mergeConversation([
      { bot: { id: bot.id, name: bot.settings.name }, items: [...mine, ...launches(bot.id, thread)] },
      ...otherChats.map(({ p, sessions: list }, i) => ({ bot: { id: p.bot.id, name: p.bot.settings.name }, items: [...own(list, 0, otherSends[i], otherQueued[i], p.thread.id), ...launches(p.bot.id, p.sessions)] })),
    ], conversation.mainId);
  }, [sessions, cards, first, sends, queued, threadId, multi, conversation, otherChats, launched, bot.id, bot.settings.name, thread, ...otherSends, ...otherQueued]); // eslint-disable-line react-hooks/exhaustive-deps
  // A message sent from this thread that continued in a new one (its session could not go on): the chat follows it.
  const followed = useRef(new Set<string>());
  useEffect(() => {
    const moved = (sends ?? []).find((m) => m.asked_thread === threadId && m.thread !== threadId && sentHere.has(m.id) && !followed.current.has(m.id));
    if (moved) { followed.current.add(moved.id); onThread?.(moved.thread); }
  }, [sends, threadId, onThread]);
  const step = useMemo(() => { const w = items.findLast((i) => i.t === "working" && (!i.bot || i.bot.id === bot.id)); return w?.t === "working" ? w.step : null; }, [items, bot.id]);
  // The records behind each steps line's receipts, one stable array per line (rows are memoized on it).
  // A line keeps its array while its records are the same, so a streaming answer elsewhere never redraws it.
  const lastStepRecords = useRef(new Map<string, readonly (ChangeRecord | undefined)[]>());
  const stepRecords = useMemo(() => {
    const next = new Map(items.flatMap((i) => {
      if (i.t !== "steps" || !i.changes.length) return [];
      const fresh = i.changes.map((c) => records.get(c.id));
      const old = lastStepRecords.current.get(i.key);
      return [[i.key, old && old.length === fresh.length && old.every((r, n) => r === fresh[n]) ? old : fresh] as const];
    }));
    lastStepRecords.current = next;
    return next;
  }, [items, records]);
  useEffect(() => { onStep?.(step); }, [step, onStep]);
  const more = from > 0 || !!(ready[0] && logs[ready[0].id]?.earlier);
  const failedLog = visible.map((s) => logs[s.id]).find((l) => l?.error);

  // The chat shows the current session as Agents does: no notification about it, and seen after 1.5 s of use.
  const focused = useWindowFocused();
  const shownCurrent = current && logs[current.id]?.loaded ? current : null;
  const shownId = shownCurrent?.id ?? null;
  // In a conversation, the other bots' current sessions show too.
  const otherCurrent = useMemo(() => others.map((p) => [...p.sessions].reverse().find((s) => s.status !== "closed") ?? null).filter((s): s is Session => !!s && !!logs[s.id]?.loaded), [others, logs]);
  const presenceKey = [shownId, ...otherCurrent.map((s) => s.id)].filter(Boolean).join(",");
  useEffect(() => { sendPresence(view, focused, presenceKey ? presenceKey.split(",") : []); }, [view, focused, presenceKey]);
  useEffect(() => () => sendPresence(view, false, []), [view]);
  useMarkSeen(shownCurrent, focused);

  // The open question of the current session: typed text answers it, unless it asks for a permission.
  // In a conversation, the newest open question of any of its bots: the box answers that bot's session.
  const asking = useMemo(() => new Set([current?.id, ...otherCurrent.map((s) => s.id)].filter(Boolean)), [current?.id, otherCurrent]);
  const ask = useMemo((): Extract<Reply, { kind: "ask" }> | null => {
    for (let i = items.length - 1; i >= 0; i--) {
      const it = items[i];
      if (it.t === "ask" && asking.has(it.sessionId) && !it.row.answers) return { kind: "ask", request_id: it.row.request_id, count: it.row.questions.length, permission: it.row.questions.some((q) => q.permission), question: it.row.questions[0]?.question ?? "", sessionId: it.sessionId, bot: multi ? it.bot?.name ?? null : null };
    }
    return null;
  }, [items, asking, multi]);
  const askSession = useMemo(() => [current, ...otherCurrent].find((s) => s?.id === ask?.sessionId) ?? null, [current, otherCurrent, ask?.sessionId]);
  // What the box answers: a question report the owner picked Reply on, else the open question, unless cancelled.
  const [report, setReport] = useState<Extract<Reply, { kind: "report" }> | null>(null);
  const [skipped, setSkipped] = useState<string | null>(null);
  const reportWaits = report && byProposal.get(report.id)?.state === "proposed";
  const reply: Reply | null = reportWaits ? report : ask && ask.request_id !== skipped ? ask : null;
  const onReply = useCallback((p: Proposal) => setReport({ kind: "report", id: p.id, title: p.title }), []);
  // Continue only on the last turn of the current session, when it failed or was interrupted.
  const continueKey = useMemo(() => {
    if (!current || (current.status !== "error" && current.status !== "interrupted")) return null;
    const last = items.findLast((i) => i.t === "ended" || i.t === "user" || i.t === "from" || i.t === "wake" || i.t === "answer" || i.t === "routine");
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
  // Scrolling up loads earlier messages. A wheel or swipe at the very top fires no scroll event, so check again
  // once a load ended or rows changed: still near the top (or the rows do not fill the view) loads the next part.
  const loadMoreRef = useRef(loadMore);
  loadMoreRef.current = loadMore;
  const settled = more && !loadingMore && ready.length === visible.length;
  useEffect(() => {
    if (!settled) return;
    const frame = requestAnimationFrame(() => { if ((scrollRef.current?.scrollTop ?? Infinity) < 150) loadMoreRef.current(); });
    return () => cancelAnimationFrame(frame);
  }, [settled, items.length]);

  return (
    <ReplyContext.Provider value={onReply}>
    <div className="flex h-full min-h-0 min-w-0 flex-1 flex-col bg-paper" data-tour="bots.thread">
      {multi && conversation && conversationBar && <ConversationBar conversation={conversation} />}
      {otherCurrent.map((s) => <SeenMark key={s.id} session={s} focused={focused} />)}
      {failedLog && <div role="alert" className="flex flex-wrap items-center gap-3 px-4 py-2 text-[12px] text-ink-3">
        <span className="min-w-0 flex-1 break-words">Could not load the chat: {failedLog.error}</span>
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
          {thread.length === 0 && !items.length ? <div className="py-8">{intro}</div> : <>
            {more && <div className="flex justify-center pt-2"><button className="btn btn-sm" disabled={loadingMore} onClick={loadMore}>{loadingMore ? "Loading…" : "Load earlier messages"}</button></div>}
            {thread.length > 0 && sessions.length === 0 && <div className="p-6 text-center text-[13px] text-ink-3">Loading…</div>}
            <div style={{ height: total, position: "relative" }}>
              {v.getVirtualItems().map((it) => {
                const item = items[it.index];
                const data = item.t === "card" ? item.kind === "proposal" ? byProposal.get(item.id) : item.kind === "ready" ? byItem.get(item.id) : records.get(item.id) : item.t === "steps" ? stepRecords.get(item.key) : undefined;
                const extra = item.t === "card" && item.kind === "ready" && data ? bySuggestion.get((data as TodayItem).record) : undefined;
                return (
                  <div key={it.key} data-index={it.index} ref={v.measureElement} className="absolute top-0 right-0 left-0" style={{ transform: `translateY(${it.start}px)` }}>
                    <ChatRow item={item} data={data} extra={extra} canContinue={item.key === continueKey} botId={item.bot?.id ?? bot.id} botName={item.bot?.name ?? bot.settings.name} quietWork={!!onStep && !item.bot} />
                  </div>
                );
              })}
            </div>
          </>}
          <div className="h-3" />
        </div>
      </div>
      <BotComposer key={`${bot.id}.${threadId}`} bot={bot} threadId={threadId} onThread={onThread} current={current} askSession={askSession} conversation={conversation} rotation={rotation} needsProfile={needsProfile} openSettings={openSettings} reply={reply} skipped={!reportWaits && ask && ask.request_id === skipped ? ask : null}
        onCancelReply={() => { if (reply?.kind === "report") setReport(null); else if (reply) setSkipped(reply.request_id); }} onUnskip={() => setSkipped(null)}
        onSent={() => { atBottom.current = true; setReport(null); }} />
    </div>
    </ReplyContext.Provider>
  );
}

// ---------------------------------------------------------------- rows

const QUEUED_FROM: Record<string, string> = { telegram: "Telegram", hook: "a hook", channel: "a channel", agent: "an agent" };

/** A hook, Telegram, channel or agent message waiting for the thread's turn to end (or for a free slot), with Cancel. */
function QueuedRow({ item, botId }: { item: Extract<ChatItem, { t: "queued" }>; botId: string }) {
  const qc = useQueryClient();
  return <div className="py-1" data-tour="chat.queued"><div className="flex items-center gap-2 rounded-card border border-dashed border-line px-2.5 py-1.5 text-[13px] text-ink-3">
    <Icon name="clock" size={12} className="flex-none" />
    <span className="min-w-0 flex-1">
      <span className="line-clamp-2 break-words whitespace-pre-wrap">{item.text}</span>
      <span className="block text-[11px]">Waiting · from {QUEUED_FROM[item.source] ?? item.source} · {ago(item.at)} · {item.slot ? "waiting for a free slot: the bot works in as many chats as it may at once" : "sent when the current turn ends"}</span>
    </span>
    <ActionButton name="bot.cancel-queued" params={{ id: botId, n: String(item.n) }} icon={null} onDone={() => qc.invalidateQueries({ queryKey: ["bot-queued", botId] })} />
  </div></div>;
}

/** How far your accepted message got (`BotSend.state`), under its bubble until its session shows it. */
const PENDING: Record<string, string> = {
  waiting: "Queued: sent once the bot can take it",
  sending: "Sending…",
  sent: "Starting…",
};

/** Your message Home accepted, before its session shows it: a faded bubble that says how far it got (queued,
 * sending, starting), replaced by the session's own message once the bot gets it; a failed or uncertain one keeps
 * its bubble with why, Retry, and Remove in ⋯. */
function PendingRow({ item, botId }: { item: Extract<ChatItem, { t: "pending" }>; botId: string }) {
  const qc = useQueryClient();
  const refresh = () => qc.invalidateQueries({ queryKey: ["bot-sends", botId] });
  const stuck = item.state === "failed" || item.state === "uncertain";
  const images = item.files.filter((f) => isImageAttachment(f.data_url));
  const others = item.files.filter((f) => !isImageAttachment(f.data_url));
  return <ChatMessage tour="chat.pending" look="owner" faded={!stuck} avatar={<OwnerAvatar />} name="You" after={stuck
      ? <div className="flex max-w-full items-center gap-2 text-[12px] text-gh-closed">
        <Icon name="xcircle" size={12} className="flex-none" />
        <span className="min-w-0 flex-1 break-words">{item.state === "failed" ? "Not sent" : "Not confirmed"}: {item.error}</span>
        <RowActions width={0} label="More actions" main={<ActionButton name="bot.retry-send" params={{ id: botId, send: item.id }} icon={null} onDone={refresh} />}
          more={(close) => <ActionMenuItem name="bot.remove-send" params={{ id: botId, send: item.id }} close={close} onDone={refresh} detail="It is never sent" />} />
      </div>
      : <span className="flex items-center gap-1.5 text-[11px] text-ink-3" role="status"><Icon name="clock" size={12} />{PENDING[item.state] ?? item.state}</span>}>
    <div className="whitespace-pre-wrap">{item.text}</div>
    {images.length > 0 && <div className="flex flex-wrap gap-2">{images.map((image, i) => <ImageThumb key={i} src={attachmentSrc(image.data_url)} name={image.name} className="mt-2 max-h-40 max-w-full rounded-control border border-line" />)}</div>}
    {others.length > 0 && <span className="mt-1 block text-[11px] text-ink-3">{others.map((f) => f.name).join(", ")}</span>}
  </ChatMessage>;
}

type RowData = Proposal | TodayItem | ChangeRecord | readonly (ChangeRecord | undefined)[] | undefined;

const ChatRow = memo(function ChatRow({ item, data, extra, canContinue, botId, botName, quietWork }: { item: ChatItem; data: RowData; extra: Suggestion | undefined; canContinue: boolean; botId: string; botName: string; quietWork: boolean }) {
  switch (item.t) {
    case "divider":
      return <div className="pt-4 pb-2"><Separator>
        <span>{item.index > 1 ? `Session ${item.index} · started` : "Started"} {when(item.at)}{item.previous ? ` · continues “${item.previous}”` : ""}{item.reason ? ` because ${item.reason}` : ""}</span>
      </Separator></div>;
    case "user":
      return <ChatMessage look="owner" avatar={<OwnerAvatar />} name="You" tag={item.to ? <SenderTag title="The bots of this conversation that got your message"><Recipients names={item.to} /></SenderTag> : undefined}>
        {item.text && <div className="whitespace-pre-wrap">{item.text}</div>}
        {item.images.length > 0 && <div className="flex flex-wrap gap-2">{item.images.map((image, i) => { const src = attachmentSrc(image.data_url); return <ImageThumb key={i} src={src} name={image.name} className="mt-2 max-h-40 max-w-full rounded-control border border-line" />; })}</div>}
        {item.queued && <span className="ml-2 text-[11px] text-ink-3">queued: read after the current step</span>}
        {item.instructions && <span className="mt-1 flex items-center gap-1 text-[11px] text-ink-3" title="BOT.md changed since this session last read it: the new BOT.md went with this message"><Icon name="file" size={12} />With your updated BOT.md</span>}
      </ChatMessage>;
    case "from":
      return <FromRow item={item} />;
    case "wake":
      return <WakeRow item={item} />;
    case "sleeping":
      return <SleepingRow item={item} />;
    case "routine":
      return <div className="py-1.5"><Separator><span className="flex min-w-0 items-center gap-1.5" title="A routine started this run"><Icon name="refresh" size={12} className="flex-none" /><span className="min-w-0">Routine “<RoutineName title={item.title} />” · {item.cause}</span></span></Separator></div>;
    case "quiet":
      return <div className="flex items-center gap-1.5 py-1 text-[12px] text-ink-3" title="The run had nothing to report: no notification"><Icon name="refresh" size={12} /><span className="truncate"><RoutineName title={item.title} /> · {item.cause}{item.at ? ` · ${when(item.at)}` : ""} · no update</span></div>;
    case "answer":
      return <ChatMessage look="bot" edges={item.to?.length ? { from: botName, to: item.to } : undefined} avatar={<BotAvatar name={botName} />} name={<span className={botInk(botName)}>{item.bot ? <Link to="/bots/$id" params={{ id: item.bot.id }} className={NAME_LINK}>{botName}</Link> : botName}</span>}
        tag={item.streaming ? <SenderTag><span className="text-work">writing…</span></SenderTag> : item.to ? <SenderTag title="Its answer goes back to the bot that asked"><Recipients names={item.to} /></SenderTag> : undefined}>
        <AssistantRow text={item.text} streaming={item.streaming} />
      </ChatMessage>;
    case "steps":
      return <StepsRow item={item} records={(data ?? []) as readonly (ChangeRecord | undefined)[]} />;
    case "ask":
      return <div className="py-1.5">{item.bot && <div className={`pb-1 text-[12px] text-ink-3 ${MESSAGE_INDENT}`}><span className={`font-semibold ${botInk(item.bot.name)}`}>{item.bot.name}</span> asks</div>}<AskLive item={item} /></div>;
    case "launch":
      return <LaunchRow item={item} />;
    case "ended":
      return <EndedRow item={item} canContinue={canContinue} />;
    case "card":
      // A change of a turn without a steps line (it still works, or a quiet run): its full line.
      if (item.kind === "changed") return <div className="flex min-h-7 items-center gap-2 py-1 text-[12px] text-ink-2" data-tour="chat.changed"><ChangeLine record={data as ChangeRecord | undefined} text={item.text ?? ""} /></div>;
      if (!data) return null;
      return <div className="grid min-w-0 grid-cols-[minmax(0,1fr)] py-1.5">{item.kind === "proposal" ? <ProposalRow p={data as Proposal} /> : <ReadyLine item={data as TodayItem} suggestion={extra} />}</div>;
    case "changes":
      return <ChangesRow sessionId={item.sessionId} />;
    case "working":
      return <WorkingRow item={item} quiet={quietWork} />;
    case "queued":
      return <QueuedRow item={item} botId={botId} />;
    case "pending":
      return <PendingRow item={item} botId={botId} />;
  }
}, (a, b) => a.item.key === b.item.key && a.item.v === b.item.v && a.data === b.data && a.extra === b.extra && a.canContinue === b.canContinue && a.botId === b.botId && a.botName === b.botName && a.quietWork === b.quietWork);

/** A proposal as a compact line; a question report's Reply aims the chat's message box at it. */
function ProposalRow({ p }: { p: Proposal }) {
  const onReply = useContext(ReplyContext);
  return <ProposalLine p={p} onReply={onReply} />;
}

/** A ready click the turn prepared (an Outbox message, a Suggestion, a Local PR) as one line, like a proposal: its
 * title and what it is, Review and ⋯. Review shows the full card in place (a Local PR opens its page), where Send or
 * Post runs with its confirmation. */
function ReadyLine({ item, suggestion }: { item: TodayItem; suggestion: Suggestion | undefined }) {
  const [open, setOpen] = useState(false);
  const phone = usePhone();
  const look = LOOK[item.kind];
  const ref = githubRef(item);
  const url = suggestion ? suggestionItemUrl(suggestion) : ref ? githubItemUrl(ref.repo, ref.number) : null;
  if (open) return <div className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-1">
    <ItemCard item={item} proposal={undefined} suggestion={suggestion} />
    <button className="chat-tap justify-self-start text-[12px] text-teal-ink hover:underline" onClick={() => setOpen(false)}>Hide details</button>
  </div>;
  const pr = item.kind === "local_pr";
  const main = pr
    ? <Link className="btn btn-sm no-underline" to="/predrafts/$id" params={{ id: item.record }} title="Publish to GitHub or merge locally from its page">Review</Link>
    : <button className="btn btn-sm" title="Read it, then send or post it" onClick={() => setOpen(true)}>Review</button>;
  const more = (pr || !!url) && ((close: () => void) => <>
    {pr && <MenuItem icon={<Icon name="expand" size={14} />} label="Details" detail="The full card" onClick={() => { close(); setOpen(true); }} />}
    {url && <MenuItem icon={<Icon name="ext" size={14} />} label="Open on GitHub" onClick={() => { close(); openExternal(url); }} />}
  </>);
  return (
    <article data-tour="chat.ready" data-tour-id={item.id} className="group grid min-w-0 grid-cols-[minmax(0,1fr)] gap-0.5 rounded-card border border-line bg-paper py-1.5 pr-1.5 pl-2.5">
      <div className="flex min-w-0 items-center gap-2">
        <Icon name={look.icon} size={14} className="flex-none text-ink-3" />
        <span className="min-w-0 flex-1">
          <b className={`block text-[13px] ${phone ? "line-clamp-2" : "truncate"}`} title={item.title}>{item.title}</b>
          <span className={`block text-[12px] text-ink-3 ${phone ? "line-clamp-2" : "truncate"}`} title={look.name}>{item.detail || look.name}</span>
        </span>
        <RowActions width={phone ? 0 : undefined} label="More actions" main={main} more={more} />
      </div>
    </article>
  );
}

/** The full line of a change: "Changed", then what changed with the values before, the record's name linking to it. */
function ChangeLine({ record, text }: { record: ChangeRecord | undefined; text: string }) {
  const a = record?.kind === "act" ? record.a : undefined;
  return <>
    <span className="flex-none rounded bg-sunk px-1.5 text-[11px] font-semibold text-ink-3">Changed</span>
    <span className="min-w-0 flex-1 truncate" title={a ? `${(ACTIONS as Record<string, { help: string }>)[a.action]?.help ?? ""}: ${text}` : text}>
      {a ? <ActivityText a={a} /> : record?.kind === "kb" ? <KbText c={record.c} /> : record?.kind === "bot" ? <BotFileText c={record.c} botId={record.botId} /> : record?.kind === "folio" ? <ReceiptLink record={record}>{text}</ReceiptLink> : text}
    </span>
  </>;
}

/** Where a change receipt leads: the note, the bot's About, or the changed record's page. */
function ReceiptLink({ record, children }: { record: ChangeRecord | undefined; children: React.ReactNode }) {
  if (record?.kind === "kb" && record.c.new_sha256) return <Link to="/knowledge" search={{ note: record.c.path }} className={NAME_LINK}>{children}</Link>;
  if (record?.kind === "bot") return <Link to="/bots/$id/settings" params={{ id: record.botId }} className={NAME_LINK}>{children}</Link>;
  if (record?.kind === "folio") return <Link to="/folios/$slug" params={{ slug: record.f.slug }} className={NAME_LINK}>{children}</Link>;
  const link = record?.kind === "act" ? activityLink(record.a) : null;
  return link ? <Link {...link} className={NAME_LINK}>{children}</Link> : <>{children}</>;
}

/** "to Writer", "to Writer and Scout", "to A, B and C": each name in its bot's identity color; long lines
 * wrap between words on a phone. */
function Recipients({ names }: { names: readonly string[] }) {
  return <span className="min-w-0 break-words">to {names.map((n, i) => <Fragment key={i}>
    {i > 0 && (i === names.length - 1 ? " and " : ", ")}
    <span className={`font-semibold ${botInk(n)}`}>{n}</span>
  </Fragment>)}</span>;
}

const FROM_ICON = { session: "agents", hook: "download", channel: "comment", wake: "sun", routine: "refresh", bot: "agents" } as const;

/** A message from another bot or agent, a hook or a channel (Home stamped its sender): under the sender's mark and
 * name with "not from you", in a dashed bubble, never as yours. */
function FromRow({ item }: { item: Extract<ChatItem, { t: "from" }> }) {
  const f = item.from;
  const { name, tag } = senderParts(f);
  const bot = f.kind === "bot" || (f.kind === "channel" && !!f.by?.trim());
  const link = f.kind === "bot"
    ? <Link to="/bots/$id" params={{ id: f.id }} className={NAME_LINK}>{name}</Link>
    : f.kind === "hook"
      ? <Link to="/settings/connections" className={NAME_LINK}>{name}</Link>
      : f.kind === "channel"
        ? <Link to="/channels/$id" params={{ id: f.id }} className={NAME_LINK}>{name}</Link>
        : <Link to="/agents" search={{ session: f.id }} className={NAME_LINK}>{name}</Link>;
  // A bot's message to other bots of this conversation: "Scout to Writer", both names in their colors, with thin
  // edges in those colors instead of the dashed "not from you" bubble (the tooltip keeps the provenance).
  const between = f.kind === "bot" && item.to?.length ? item.to : null;
  return <ChatMessage tour="chat.from" look={between ? "bot" : "other"} edges={between ? { from: name, to: between } : undefined}
    avatar={bot ? <BotAvatar name={name} /> : <SourceAvatar icon={FROM_ICON[f.kind]} />}
    name={<span className={bot ? botInk(name) : "text-ink"}>{link}</span>}
    tag={<SenderTag title="Launchpad stamped the sender: the bot read this message as information, not as your instructions">{between ? <Recipients names={between} /> : item.to ? `→ ${item.to.join(", ")} · not from you` : tag}</SenderTag>}>
    <div className="break-words whitespace-pre-wrap">{item.text}</div>
    {item.images.length > 0 && <div className="flex flex-wrap gap-2">{item.images.map((image, i) => { const src = attachmentSrc(image.data_url); return <a key={i} href={src} download={image.name} title={`Save ${image.name}`}><img src={src} alt={image.name} className="mt-2 max-h-40 max-w-full rounded-control border border-line" /></a>; })}</div>}
    {item.queued && <span className="text-[11px] text-ink-3">queued: read after the current step</span>}
    {item.instructions && <span className="mt-1 flex items-center gap-1 text-[11px] text-ink-3" title="BOT.md changed since this session last read it: the new BOT.md went with this message"><Icon name="file" size={12} />With your updated BOT.md</span>}
  </ChatMessage>;
}

const WAKE_EARLY = { timeout: "its timeout passed first", owner: "you woke it early" } as const;

/** The wake-up the agent asked for (`launchpad sleep`): one quiet line, "Woken up · today 15:00", never a claim that
 * what it waited for happened (only Home's own words: its timeout, or your Wake now). Launchpad's message to the
 * agent (when, how to continue, its note) is behind Details. */
function WakeRow({ item }: { item: Extract<ChatItem, { t: "wake" }> }) {
  const [open, setOpen] = useState(false);
  return <div className="py-1" data-tour="chat.wake">
    <div className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5 text-[12px] text-ink-3">
      <Icon name="sun" size={12} className="flex-none" />
      <span className="text-ink-2">Woken up</span>
      {item.at != null && <span title={when(item.at)}>· {when(item.at)}</span>}
      {item.early && <span>· {WAKE_EARLY[item.early]}</span>}
      {item.queued && <span>· waiting for the current step</span>}
      <button className="chat-tap flex flex-none items-center gap-1 hover:text-ink" aria-expanded={open} title="The message Launchpad sent to wake it" onClick={() => setOpen(!open)}>
        · <Icon name="chev" size={12} className={`transition-transform ${open ? "" : "-rotate-90"}`} />Details
      </button>
    </div>
    {open && <div className="mt-1 border-l-2 border-line-2 pl-3 font-mono text-[11px] break-words whitespace-pre-wrap text-ink-3">{item.text}</div>}
  </div>;
}

/** The session sleeps now: "Sleeping until today 15:00", its note, Wake now and Cancel wake-up behind Details. */
function SleepingRow({ item }: { item: Extract<ChatItem, { t: "sleeping" }> }) {
  const [open, setOpen] = useState(false);
  const z = item.sleep;
  const until = sleepUntil(z);
  return <div className="py-1.5" data-tour="chat.sleeping">
    <div className="rounded-card bg-sunk px-2.5 py-1.5 text-[12px] text-ink-2">
      <div className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5">
        <Icon name="moon" size={12} className="flex-none text-ink-3" />
        <span className="min-w-0 break-words"><b className="font-semibold">Sleeping</b>{until ? ` ${until}` : ""}</span>
        <button className="chat-tap ml-auto flex flex-none items-center gap-1 text-ink-3 hover:text-ink" aria-expanded={open} title="Its note, and what you can do" onClick={() => setOpen(!open)}>
          <Icon name="chev" size={12} className={`transition-transform ${open ? "" : "-rotate-90"}`} />Details
        </button>
      </div>
      {open && <div className="mt-1.5 grid gap-1.5 border-t border-line-2 pt-1.5">
        <span className="text-ink-3">Launchpad wakes it in this conversation{z.deadline != null ? ", at the latest at its timeout even if it still waits" : ""}.</span>
        {z.note.trim() && <span className="break-words whitespace-pre-wrap"><span className="text-ink-3">Then: </span>{z.note.trim()}</span>}
        <div className="flex flex-wrap items-center gap-2">
          <ActionButton name="schedule.start" params={{ id: z.schedule_id }} icon={null}>Wake now</ActionButton>
          <ActionButton name="schedule.cancel" params={{ id: z.schedule_id }} icon={null} consequence="The agent stays Done and is not woken up.">Cancel wake-up</ActionButton>
        </div>
      </div>}
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

/** A change of the bot's own files: the path opens the bot's About. */
function BotFileText({ c, botId }: { c: Change; botId: string }) {
  return <>{kbVerb(c)} <Link to="/bots/$id/settings" params={{ id: botId }} className={NAME_LINK}>{c.path}</Link>{kbStats(c)}</>;
}

/** A direct change: the record's name opens its page. */
function ActivityText({ a }: { a: Activity }) {
  const p = activityParts(a);
  const link = activityLink(a);
  return <>{p.before}{p.target && (link ? <Link {...link} className={NAME_LINK}>{p.target}</Link> : p.target)}{p.after}</>;
}

/** One quiet line per turn: "12 steps · 3 min", then a receipt per change ("noted in its memory", "changed routine
 * Daily check") linking to its record. A tap lists the step titles and the full change lines inline; the trace
 * itself stays in Agents. */
function StepsRow({ item, records }: { item: Extract<ChatItem, { t: "steps" }>; records: readonly (ChangeRecord | undefined)[] }) {
  const [open, setOpen] = useState(false);
  return <div className={`py-1 ${MESSAGE_INDENT}`}>
    <div className="flex min-w-0 items-center gap-1.5 text-[12px] text-ink-3" data-tour={item.changes.length ? "chat.changed" : undefined}>
      <button className="chat-tap flex flex-none items-center gap-1 hover:text-ink" aria-expanded={open} title="The work behind this answer, one line per step" onClick={() => setOpen(!open)}>
        <Icon name="chev" size={12} className={`transition-transform ${open ? "" : "-rotate-90"}`} />{item.bot ? `${item.bot.name} · ` : ""}{stepsLabel(item.count, item.durationMs)}
      </button>
      {item.bot && <Link to="/agents" search={{ session: item.sessionId }} className="flex-none text-inherit hover:underline" title={`Open ${item.bot.name}'s trace in Agents`}>trace</Link>}
      {item.changes.length > 0 && <span className="flex min-w-0 items-center gap-1.5 truncate">
        {item.changes.map((c, i) => <span key={c.id} className="truncate" title={c.text}>· <Icon name="pencil" size={12} className="inline align-[-2px]" /> <ReceiptLink record={records[i]}>{c.receipt}</ReceiptLink></span>)}
      </span>}
    </div>
    {open && <div className="mt-1 grid gap-0.5 border-l-2 border-line-2 pl-3">
      <ol className="m-0 grid gap-0.5 p-0 text-[12px] text-ink-3">{item.steps.map((s, i) => <li key={i} className="list-none truncate font-mono text-[11px]" title={s}>{s}</li>)}</ol>
      {item.changes.map((c, i) => <div key={c.id} className="flex min-h-6 items-center gap-2 text-[12px] text-ink-2"><ChangeLine record={records[i]} text={c.text} /></div>)}
    </div>}
  </div>;
}

/** A question of the bot: its options as chips, answered by a chip or from the message box. */
function AskLive({ item }: { item: Extract<ChatItem, { t: "ask" }> }) {
  const live = useSession(item.sessionId)?.live ?? false;
  return <AskCard row={item.row} sessionId={item.sessionId} live={live} chips />;
}

function EndedRow({ item, canContinue }: { item: Extract<ChatItem, { t: "ended" }>; canContinue: boolean }) {
  const [busy, setBusy] = useState(false);
  return <div className={`flex flex-wrap items-center gap-2 py-1.5 text-[12px] text-gh-closed ${MESSAGE_INDENT}`}>
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

/** The turn at work, with View running agent (this thread's session in Agents) and Stop. `quiet`: the page's header
 * already says what it works on and since when, so the row shows only the spinner, the link and Stop. The row exists
 * only once the session record does: a message still waiting for its launch is a pending bubble, never a link. */
function WorkingRow({ item, quiet }: { item: Extract<ChatItem, { t: "working" }>; quiet: boolean }) {
  const s = useSession(item.sessionId);
  const [busy, setBusy] = useState(false);
  const [, tick] = useState(0);
  useEffect(() => { const t = setInterval(() => tick((n) => n + 1), 5_000); return () => clearInterval(t); }, []);
  if (!s || !isRunning(s)) return null;
  const label = s.status === "starting" ? "Starting" : s.status === "background" ? "Background work" : "Working";
  return <div className={`flex items-center gap-2 py-1.5 text-[12px] text-work ${MESSAGE_INDENT}`} role="status" aria-label={quiet ? label : undefined}>
    <span className="lp-spin" />
    {!quiet && <span className="min-w-0 truncate">{`${item.bot ? `${item.bot.name} · ` : ""}${label}${item.step ? ` · ${item.step}` : ""} · ${age(s.status_since)}`}</span>}
    <Link to="/agents" search={{ session: s.id }} className="shrink-0 text-inherit underline underline-offset-2" title="Open this chat's session in Agents: its trace, terminal and files">View running agent</Link>
    <span className="flex-1" />
    <button className="btn btn-sm" disabled={busy} title="Stop: interrupt the bot" onClick={() => { setBusy(true); void runAction("session.stop", { id: s.id }).catch(toastError).finally(() => setBusy(false)); }}><Icon name="stop" size={12} />Stop</button>
  </div>;
}

/** An agent a bot of the conversation launched: one line linking to it in Agents. */
function LaunchRow({ item }: { item: Extract<ChatItem, { t: "launch" }> }) {
  const s = useSession(item.sessionId);
  const status = s ? statusWord(s) : null;
  return <div className={`flex min-h-7 items-center gap-1.5 py-1 text-[12px] text-ink-3 ${MESSAGE_INDENT}`}>
    <Icon name="agents" size={12} className="flex-none" />
    <span className="min-w-0 truncate">{item.bot ? <span className={`font-semibold ${botInk(item.bot.name)}`}>{item.bot.name}</span> : "The bot"} launched <Link to="/agents" search={{ session: item.sessionId }} className={NAME_LINK}>{s?.title ?? item.title}</Link>{status ? ` · ${status}` : ""}</span>
  </div>;
}

const statusWord = (s: Session) => isRunning(s) ? "working" : s.open_question ? "waits for you" : s.status === "error" ? "failed" : s.status === "interrupted" ? "interrupted" : s.status === "closed" ? "archived" : isSleeping(s) ? "sleeping" : "done";

/** The bots of a conversation of several bots, over its timeline: each bot's mark (its trace in Agents) and one
 * status line, the most urgent first ("Docs working · Scout waits for you · Release waiting"). */
function ConversationBar({ conversation }: { conversation: ConversationInfo }) {
  const parts = conversation.participants.map((p) => ({ p, session: [...p.sessions].reverse().find((s) => s.status !== "closed") ?? null }));
  const status = conversationStatus(parts.map(({ p, session }) => ({ name: p.bot.settings.name, session })));
  return <div className="flex min-h-9 flex-none items-center gap-2 border-b border-line-2 px-3 py-1" data-tour="chat.conversation">
    <div className="mx-auto flex w-full max-w-text min-w-0 items-center gap-2">
      <span className="flex flex-none items-center gap-1">
        {parts.map(({ p, session }) => session
          ? <Link key={p.bot.id} to="/agents" search={{ session: session.id }} title={`Open ${p.bot.settings.name}'s trace in Agents`} aria-label={`Open ${p.bot.settings.name}'s trace`}><BotAvatar name={p.bot.settings.name} size={20} /></Link>
          : <span key={p.bot.id} title={p.bot.settings.name}><BotAvatar name={p.bot.settings.name} size={20} /></span>)}
      </span>
      <span className="min-w-0 truncate text-[12px] text-ink-3" role="status">{status}</span>
    </div>
  </div>;
}

/** Marks a session of another bot of the conversation seen, like the thread's own (`useMarkSeen`). */
function SeenMark({ session, focused }: { session: Session; focused: boolean }) {
  useMarkSeen(isUnseen(session) ? session : null, focused);
  return null;
}

// ---------------------------------------------------------------- the message box

const firstLine = (t: string) => t.trim().split("\n")[0] ?? "";

function BotComposer({ bot, threadId, onThread, current, askSession, conversation, rotation, needsProfile, openSettings, reply, skipped, onCancelReply, onUnskip, onSent }: {
  bot: Bot; threadId: string; onThread?: (thread: string) => void; current: Session | null; rotation: string | null; needsProfile: boolean;
  /** The session whose open question the box answers (in a conversation, any of its bots'), else the thread's. */
  askSession: Session | null;
  /** The thread's conversation: the box sends to its main bot, or to the bots its leading @-mentions name. */
  conversation: ConversationInfo | null;
  openSettings: () => void;
  /** What the box answers (the open question, or a question report), else null: a normal message. */
  reply: Reply | null;
  /** The open question the owner cancelled answering: a normal message closes it as skipped first. */
  skipped: Extract<Reply, { kind: "ask" }> | null;
  onUnskip: () => void;
  onCancelReply: () => void;
  onSent: () => void;
}) {
  const qc = useQueryClient();
  const harness = useHarnessInfo(current?.harness ?? null);
  const acceptsImages = harness ? harness.capabilities.images : true;
  // The unsent text survives the box being hidden (the phone's About page, back, another bot in the Ask panel).
  // One draft per thread: the box is mounted again for each thread (its `key`), so a draft never follows to another.
  const { text, setText } = useKeptDraft(chatDraftKey(bot.id, threadId), "");
  // Where the message goes: the thread shown, or a new thread.
  const where = threadId === NEW_THREAD ? { new_thread: true } : { thread: threadId };
  const [busy, setBusy] = useState(false);
  // A message whose response was lost: sending it unchanged returns its result (the same Idempotency-Key), never a
  // second message; only the owner's click after checking the chat starts a new attempt. The exact body is kept, so
  // a retry stays the same request even when the lost message already started a new thread.
  const messagePath = `/api/bots/${bot.id}/message`;
  const [lost, setUnconfirmed] = useState<{ path: string; body: Record<string, unknown>; message: string } | null>(null);
  const unconfirmed = lost?.path === messagePath ? lost : null;
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
  const asker = askSession ?? current;
  const ask = reply?.kind === "ask" && !!asker?.open_question && !!asker.live ? reply : null;
  const report = reply?.kind === "report" ? reply : null;
  const answering = !!ask || !!report;
  const skip = !answering && skipped && !skipped.permission && !!asker?.open_question && !!asker.live ? skipped : null;
  // Leading @-mentions route the message (a conversation's thread); an unknown name is refused before it is sent.
  const routed = !answering && !!conversation && threadId !== NEW_THREAD;
  const [mentionError, setMentionError] = useState<string | null>(null);
  const query = routed ? mentionQuery(text) : null;
  const participants = useMemo(() => conversation?.participants.map((p) => p.bot.id) ?? [], [conversation]);
  const choices = query === null || !conversation ? [] : mentionChoices(query, conversation.bots, participants);
  const mainName = conversation?.bots.find((b) => b.id === conversation.mainId)?.name ?? bot.settings.name;
  const many = participants.length > 1;
  // Cancel needs a harness that takes the message while the skipped question's turn goes on.
  const canSkip = !!harness?.capabilities.messages_while_running;
  // Typed text never grants a permission. A harness that cannot take messages while it works gets yours at the turn's
  // end: Home keeps it queued meanwhile (also when the owner cancelled answering an open question to send a message).
  const busyTurn = !ask && !!current && isRunning(current) && !!harness && !harness.capabilities.messages_while_running;
  const blocked = !!ask && ask.permission;
  const context = answering ? EMPTY_CONTEXT : storedContext;
  const presets = answering ? [] : selectedPresets.filter((id) => botPresets.some((p) => p.id === id));
  const withContext = hasMessageContext(context);
  const send = async (typed = text) => {
    const t = answering ? typed.trim() : withPresets(typed, presets, botPresets);
    if ((!t && !files.length && !withContext) || busy || reading || blocked) return;
    setBusy(true);
    // The thread the message goes to: a message from New thread lands in the thread Home creates for it.
    let to = threadId;
    try {
      if (ask && asker) {
        await runAction("session.answer", { id: asker.id }, { request_id: ask.request_id, answers: Array(ask.count).fill(t || "See the attachments in my next message.") });
        if (files.length) await accepted(await runAction<BotSend>("bot.message", { id: bot.id }, { text: "", attachments: files, ...where, accept: true }));
      } else if (report) {
        const next = await runAction<Proposal>("proposal.reply", { id: report.id }, { text: t });
        qc.setQueryData<Proposal[]>(["proposals"], (old) => old?.map((x) => (x.id === next.id ? next : x)));
        if (next.state === "failed") throw new Error(next.error ?? "The reply was not sent");
        if (files.length) await accepted(await runAction<BotSend>("bot.message", { id: bot.id }, { text: "", attachments: files, ...where, accept: true }));
      } else {
        // A cancelled question still waits in the harness, which would hold the message behind it: close it as
        // skipped (never a permission: those keep Allow and Deny), then send the message.
        if (skip && asker) await runAction("session.answer", { id: asker.id }, { request_id: skip.request_id, answers: Array(skip.count).fill("No answer: the owner skipped this question and sent a message instead.") });
        const route = routed && conversation ? routeMentions(t, conversation.bots, conversation.mainId) : null;
        if (route && "error" in route) { setMentionError(route.error); return; }
        const targets = route?.targets ?? [bot.id];
        if (targets.length !== 1 || targets[0] !== bot.id) {
          // To other bots of the conversation (a bot that is not part of it joins it), each in its own thread.
          for (const id of targets) await accepted(await runAction<BotSend>("bot.message", { id }, { text: t, attachments: files, conversation: conversation!.id, ...(withContext ? { context } : {}), accept: true }));
          if (targets.some((id) => !participants.includes(id))) void qc.invalidateQueries({ queryKey: ["bot-threads"] });
          setText("");
          dropSentChatDraft(bot.id, threadId, threadId);
          attachments.clear();
          setContext(EMPTY_CONTEXT);
          setPresets([]);
          onSent();
          return;
        }
        const message = JSON.stringify({ text: t, attachments: files, context: withContext ? context : null });
        // Home stores the message and answers at once (`accept`); the bot starts or continues in the background.
        const body = unconfirmed?.message === message ? unconfirmed.body : { text: t, attachments: files, ...where, ...(withContext ? { context } : {}), accept: true };
        let sent: BotSend;
        try { sent = await runAction<BotSend>("bot.message", { id: bot.id }, body); }
        // A write refused during Home's update never ran: not an uncertain delivery (`maintenance`).
        catch (e) { setUnconfirmed((e instanceof ApiError && e.operationCompleted) || refusedFor(e) ? null : { path: messagePath, body, message }); throw e; }
        setUnconfirmed(null);
        to = sent.thread;
        await accepted(sent);
        setContext(EMPTY_CONTEXT);
        setPresets([]);
      }
      setText("");
      dropSentChatDraft(bot.id, threadId, to);
      attachments.clear();
      onSent();
    } catch (e) { toastError(e); } finally { setBusy(false); }
  };
  // The accepted message shows at once (its bubble, in its thread), before Home's list refreshes.
  const accepted = async (sent: BotSend) => {
    showAccepted(qc, sent);
    if (sent.bot_id === bot.id && sent.thread !== threadId) onThread?.(sent.thread);
  };
  if (needsProfile)
    return <div className="border-t border-line-2 p-2.5"><div className="flex items-center gap-2 rounded-control bg-attn-soft px-3 py-2 text-[13px] text-attn">This bot has no profile.<button className="btn btn-sm ml-auto" onClick={openSettings}>Choose a profile</button></div></div>;
  return (
    <div className="border-t border-line-2 px-2.5 pt-2 pb-2.5" data-tour="bots.reply">
      <div className="mx-auto w-full max-w-text">
        {rotation && threadId !== NEW_THREAD && <div className="mb-1.5 text-[12px] text-ink-3">{rotation}: your next message continues it in a new thread of this chat, with a reference to this one.</div>}
        {threadId === NEW_THREAD && <div className="mb-1.5 flex items-center gap-1.5 text-[12px] text-ink-3"><Icon name="plus" size={12} className="flex-none" />Your next message starts a new chat with {bot.settings.name}.</div>}
        {unconfirmed && !busy && <div role="status" className="mb-1.5 flex items-center gap-1.5 text-[12px] text-ink-3"><span className="min-w-0 flex-1">Delivery not confirmed. Send the same message to check its result.</span><button className="chat-tap flex-none text-teal-ink hover:underline" title="Only if the chat does not show the message: it may then be sent twice" onClick={() => void abandonOperation(unconfirmed.path, unconfirmed.body).then(() => setUnconfirmed(null)).catch(toastError)}>I checked the chat — start a new attempt</button></div>}
        {answering && !(ask && ask.permission) && <div className="mb-1.5 flex items-center gap-1.5 text-[12px] text-ink-3" data-tour="chat.answering">
          <Icon name="comment" size={12} className="flex-none" />
          <span className="min-w-0 flex-1 truncate">{ask ? `Answering${ask.bot ? ` ${ask.bot}` : ""}` : "Replying to"}: <b className="font-semibold text-ink-2">{firstLine(ask ? ask.question : report!.title)}</b></span>
          {(report || canSkip) && <button className="chat-tap flex-none text-teal-ink hover:underline" title={ask ? "Send a normal message instead: the question is closed as skipped" : "Send a normal message instead"} onClick={onCancelReply}>Cancel</button>}
        </div>}
        {skip && <div className="mb-1.5 flex items-center gap-1.5 text-[12px] text-ink-3">
          <Icon name="comment" size={12} className="flex-none" />
          <span className="min-w-0 flex-1 truncate">Your message skips: <b className="font-semibold text-ink-2">{firstLine(skip.question)}</b></span>
          <button className="chat-tap flex-none text-teal-ink hover:underline" title="Answer the question with the message box" onClick={onUnskip}>Answer it</button>
        </div>}
        {!rotation && !answering && !skip && instructionsUpdated(bot, current) && <div className="mb-1.5 flex items-center gap-1.5 text-[12px] text-ink-3" data-tour="bots.instructions-updated" title={`BOT.md changed since ${bot.settings.name}'s current session read it: the new BOT.md goes with your next message, in the same session`}><Icon name="file" size={12} />Instructions updated · applies to your next message</div>}
        {mentionError && <div role="alert" className="mb-1.5 flex items-center gap-1.5 text-[12px] text-gh-closed"><Icon name="xcircle" size={12} className="flex-none" /><span className="min-w-0 flex-1">{mentionError}</span></div>}
        {choices.length > 0 && <div className="mb-1.5 flex flex-wrap items-center gap-1.5" role="listbox" aria-label="Bots to mention" data-tour="chat.mentions">
          {choices.map((b) => <button key={b.id} type="button" role="option" aria-selected={false} className="btn btn-sm" title={participants.includes(b.id) ? `${b.name} is in this conversation` : `${b.name} joins this conversation`} onClick={() => { setText(completeMention(text, text, b.slug)); requestAnimationFrame(() => ref.current?.focus()); }}>
            <BotAvatar name={b.name} size={16} />@{b.slug}{participants.includes(b.id) ? "" : " · joins"}
          </button>)}
        </div>}
        <input ref={fileRef} type="file" multiple hidden onChange={(e) => { void attachments.add(Array.from(e.target.files ?? [])); e.target.value = ""; }} />
        <AttachmentList files={files} disabled={busy || reading} onRemove={attachments.remove} />
        {!answering && <PresetChips surface="bots" selected={presets} onChange={setPresets} disabled={busy} className="mb-1.5" />}
        {!answering && <MessageContextChips className="mb-1.5" value={context} onChange={setContext} disabled={busy || reading} />}
        {!text && !files.length && !withContext && !presets.length && !answering && !blocked && !busy && <QuickChips starters={bot.settings.starters ?? []} onSend={(t) => void send(t)} onFill={(t) => { setText(t); requestAnimationFrame(() => { const el = ref.current; if (el) { el.focus(); el.setSelectionRange(t.length, t.length); } }); }} />}
        <div className="session-message-input flex items-end gap-2 rounded-control border border-line px-2.5 py-[7px] focus-within:border-teal focus-within:shadow-[0_0_0_3px_var(--color-teal-soft)]">
          <textarea ref={ref} rows={1} value={text} disabled={busy || blocked} aria-label={`Message ${bot.settings.name}`}
            placeholder={busyTurn ? `${bot.settings.name} is working: your message waits for the turn to end.` : blocked ? "Choose a decision on the approval card." : answering ? "Type your answer…" : many ? `Message ${mainName} · @ for another bot…` : `Message ${bot.settings.name}…`}
            onChange={(e) => { setText(e.target.value); setMentionError(null); }}
            onPaste={(e) => { if (!acceptsImages || busy) return; const pasted = pastedImages(e); if (pasted) void attachments.add(pasted); }}
            onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); void send(); } }}
            className="min-h-6 min-w-0 flex-1 resize-none bg-transparent text-[13px] leading-5 outline-none" />
          {!answering && <MessageContextButton sessionId={current?.id ?? null} value={context} onChange={setContext} presets={{ presets: botPresets, selected: presets, onChange: setPresets }} attach={{ images: acceptsImages, onPick: () => fileRef.current?.click() }} disabled={busy || reading || blocked} />}
          {answering && <button className="iconbtn" title={`Attach files${acceptsImages ? " or images" : ""}. ${ATTACH_LIMITS}`} aria-label="Attach files" disabled={busy || reading || blocked} onClick={() => fileRef.current?.click()}><Icon name="clip" size={14} /></button>}
          <button className="iconbtn border-line! bg-paper" title="Send" aria-label="Send" disabled={busy || reading || blocked || (!text.trim() && !files.length && !withContext && !presets.length)} onClick={() => void send()}><Icon name="send" size={14} /></button>
        </div>
      </div>
    </div>
  );
}


/** Profile and model (the bot's settings): a quick override of the bot's profile list and model (Access → Edit holds
 * all of its settings). A change applies to new chats (an open chat continues in a new one with its next message). */
export function BotModelChoice({ bot, current }: { bot: Bot; current: Session | null }) {
  const profiles = useProfiles();
  const qc = useQueryClient();
  const profile = profiles.find((p) => p.id === (bot.settings.profile ?? current?.launch_spec.profile ?? bot.settings.profiles?.[0]));
  const defaults = useHarnessDefaults(profile && !profile.models_url ? profile.harness : null).data?.models ?? [];
  const remote = useProfileModels(profile?.models_url);
  const models = profile?.models_url ? remote.data ?? [] : defaults;
  const running = !!current && isRunning(current);
  const save = async (model: string, chosen = bot.settings.profile) => {
    try {
      await runAction("bot.update", { id: bot.id }, { settings: { ...bot.settings, profile: chosen || (model ? profile?.id : null), model: model || null } });
      await qc.invalidateQueries({ queryKey: ["bots"] });
    } catch (e) { toastError(e); }
  };
  const field = "grid gap-1 text-[12px] font-semibold text-ink-2";
  return <div className="grid gap-2.5">
    <label className={field}>Profile
      <select className="field-input font-normal" disabled={running} value={bot.settings.profile ?? ""} onChange={(e) => void save("", e.target.value)}>
        <option value="">Automatic profile</option>
        {profiles.filter((p) => !bot.settings.profiles?.length || bot.settings.profiles.includes(p.id)).map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
      </select>
    </label>
    <label className={field}>Model
      <select className="field-input font-normal" disabled={running} value={bot.settings.model ?? ""} onChange={(e) => void save(e.target.value)}>
        <option value="">Automatic · profile default</option>
        {models.map((m) => <option key={m.id} value={m.id}>{m.display_name || m.id}</option>)}
        {bot.settings.model && !models.some((m) => m.id === bot.settings.model) && <option value={bot.settings.model}>{bot.settings.model}</option>}
      </select>
    </label>
    <span className="text-[12px] text-ink-3">{running ? "Wait for the turn to end, or Stop it." : "A change applies to new chats; an open chat continues in a new one with its next message."}</span>
  </div>;
}

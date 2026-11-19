// The Chats list (laptop and phone): every conversation of the workspace's bots and every bot channel or DM, as rows
// grouped by state like the Agents list. A conversation (`BotThread.conversation`) is one row whatever the number of
// its bots and threads (a follow-up thread keeps its conversation): its participants in the order they first appeared,
// its most urgent state and its newest activity. Channels have their own section, except unread ones (Unread) and
// archived ones (Archived). Pure, so it is tested without the app (tests/chatList.test.mjs).

import type { Bot } from "../gen/Bot";
import type { BotThread } from "../gen/BotThread";
import type { Channel } from "../gen/Channel";
import type { Session } from "../gen/Session";
import { threadOf } from "./botThread.ts";
import { mainBotOf } from "./conversations.ts";
import { isQuotaPaused, isRunning, isSleeping, isUnseen } from "./sessionStatus.ts";

/** The list's sections, in their order. */
export const CHAT_SECTIONS = ["Pinned", "Needs you", "Unread", "Running", "Quota paused", "Sleeping", "Failed", "Done", "Channels", "Archived"] as const;
export type ChatSection = (typeof CHAT_SECTIONS)[number];

export type ChatTone = "running" | "needs" | "done" | "failed" | "idle";

/** A conversation of bots: one or several bots, each with its own thread (its own session). */
export type ThreadChat = {
  kind: "thread";
  /** Its conversation (`<main bot>:<first thread>`). */
  id: string;
  /** Its pin target (`conversation:<id>`). */
  pin: string;
  /** The bot whose thread opens (the main bot when it takes part), and that thread. */
  bot: Bot;
  thread: string;
  /** Every thread of the conversation (`<bot>\n<thread>`), archived ones too: the chat a bot page shows. */
  threads: string[];
  title: string;
  /** Its bots in the order they first appeared (the main bot, then each bot as it joined). */
  participants: Bot[];
  /** The session whose state the row shows (the most urgent of its bots' current sessions), if any. */
  session: Session | null;
  /** Every session of every thread of the conversation (message search). */
  sessionIds: string[];
  label: string;
  tone: ChatTone;
  unseen: boolean;
  archived: boolean;
  /** The routine whose run started it, when it is a routine's thread. */
  routine: string | null;
  /** The newest answer of its bots, one line. */
  line: string;
  last: number;
  section: ChatSection;
};

/** A bot channel or DM. */
export type ChannelChat = {
  kind: "channel";
  id: string;
  pin: string;
  channel: Channel;
  title: string;
  /** Its member bots. */
  members: string[];
  unread: number;
  /** Messages that ask you there (its Today row). */
  asks: number;
  archived: boolean;
  line: string;
  last: number;
  section: ChatSection;
};

export type Chat = ThreadChat | ChannelChat;

const failed = (s: Session) => (s.status === "error" && s.capacity_retry_at == null) || s.status === "interrupted";
/** A finished turn the owner has not seen yet (not a question: that is Needs you). */
const unseenTurn = (s: Session) => isUnseen(s) && !s.open_question && ((s.status === "waiting" && !s.sleep) || failed(s));

/** A short status, in the words and colors of the session status pill (`statusInfo`). */
export function threadStatus(s: Session): { label: string; tone: ChatTone } {
  switch (s.status) {
    case "starting": return { label: "Starting", tone: "running" };
    case "working": return { label: "Working", tone: "running" };
    case "background": return { label: "Background", tone: "running" };
    case "waiting":
      if (s.open_question) return { label: "Question", tone: "needs" };
      if (s.sleep) return { label: "Sleeping", tone: "idle" };
      return { label: "Done", tone: "done" };
    case "quota_paused": return { label: "Quota paused", tone: "needs" };
    case "interrupted": return { label: "Interrupted", tone: "failed" };
    case "error": return s.capacity_retry_at != null ? { label: "Retry pending", tone: "idle" } : { label: "Failed", tone: "failed" };
    case "stopped": return { label: "Stopped", tone: "idle" };
    case "in_terminal": return { label: "In terminal", tone: "idle" };
    case "closed": return { label: "Archived", tone: "idle" };
  }
}

/** How urgent a session's state is, lowest first: Question > quota paused > unseen > Failed > Working > Sleeping > Done. */
function urgency(s: Session): number {
  if (s.open_question) return 0;
  if (isQuotaPaused(s)) return 1;
  if (unseenTurn(s)) return 2;
  if (failed(s)) return 3;
  if (isRunning(s)) return 4;
  if (isSleeping(s)) return 5;
  return 6;
}

/** The section of a conversation from its bots' current sessions. */
export function sectionOf(sessions: readonly Session[], archived: boolean): ChatSection {
  if (archived) return "Archived";
  if (sessions.some((s) => s.open_question)) return "Needs you";
  if (sessions.some(unseenTurn)) return "Unread";
  if (sessions.some(isRunning)) return "Running";
  if (sessions.some(isQuotaPaused)) return "Quota paused";
  if (sessions.some(isSleeping)) return "Sleeping";
  if (sessions.some(failed)) return "Failed";
  return "Done";
}

const firstLine = (text: string) => text.trim().split("\n")[0]?.trim() ?? "";

/** A session's automatic name (`titles.rs`), or "" while it has only the title its prompt gave it. */
const generatedTitle = (s: Session | null) => {
  const title = s?.title.trim() ?? "";
  return title && !firstLine(s!.prompt).startsWith(title.replace(/…$/, "").trim()) ? title : "";
};

/** A chat's title: a short automatic description, like an agent's name (`titles.rs`) — the automatic name of the
 * chat's first session (it says what the chat is about, also after the chat continued in a new thread), else of its
 * current one — for every chat, routine runs and chats a source opened (GitHub, a hook, a channel) included; the
 * thread's own title until a session is named, or when you renamed one of your chats; "New chat" before the first
 * message. */
export function chatTitle(t: Pick<BotThread, "title" | "key"> | null, first: Session | null, current: Session | null = first): string {
  const own = t?.title.trim() ?? "";
  // Your own chats keep the first line of their first message as title until you rename them.
  if (t && !t.key && own && first && !firstLine(first.prompt).startsWith(own.replace(/…$/, "").trim())) return own;
  return generatedTitle(first) || generatedTitle(current) || own || current?.title.trim() || "New chat";
}

export const isNoUpdateText = (text: string | null | undefined) => !!text && text.trim().replace(/^[`*".\s]+|[`*".\s]+$/g, "").toLowerCase() === "no_update";

/** The bots of a conversation in the order they first appeared: each bot at its first thread of the conversation
 * (`created_at`), the main bot first on a tie. "C called by you, then A, then B" reads C, A, B. */
export function participantOrder<T extends { bot_id: string; created_at: number }>(threads: readonly T[], mainId: string): string[] {
  const first = new Map<string, number>();
  for (const t of threads) first.set(t.bot_id, Math.min(first.get(t.bot_id) ?? Infinity, t.created_at));
  return [...first.entries()].sort((a, b) => a[1] - b[1] || Number(b[0] === mainId) - Number(a[0] === mainId) || a[0].localeCompare(b[0])).map(([id]) => id);
}

export type ChatInputs = {
  bots: readonly Bot[];
  sessions: readonly Session[];
  /** Each bot's threads, archived ones included (`GET /api/bots/{id}/threads?archived=true`), undefined while loading. */
  threads: ReadonlyMap<string, readonly BotThread[] | undefined>;
  channels: readonly Channel[] | undefined;
  workspaceId: string | null | undefined;
  /** Channel ID → messages that ask you there (Today's channel rows). */
  asks?: ReadonlyMap<string, number>;
};

/** Every chat of the workspace, newest activity first. */
export function buildChats({ bots, sessions, threads, channels, workspaceId, asks }: ChatInputs): Chat[] {
  const mine = new Map(bots.filter((b) => b.workspace_id === workspaceId).map((b) => [b.id, b]));
  // Each bot thread's sessions, oldest first.
  const byThread = new Map<string, Session[]>();
  for (const s of [...sessions].sort((a, b) => a.started_at - b.started_at)) {
    const bot = s.launch_spec.bot?.id;
    if (!bot || !mine.has(bot)) continue;
    const key = `${bot}\n${threadOf(s)}`;
    byThread.set(key, [...(byThread.get(key) ?? []), s]);
  }
  // Every thread by conversation: Home's lists, or (while a list loads) the threads its sessions name.
  const conversations = new Map<string, BotThread[]>();
  const add = (t: BotThread) => conversations.set(t.conversation, [...(conversations.get(t.conversation) ?? []), t]);
  for (const bot of mine.values()) {
    const list = threads.get(bot.id);
    if (list) { list.forEach(add); continue; }
    for (const [key, list2] of byThread) {
      const [b, thread] = key.split("\n");
      if (b !== bot.id) continue;
      const s = list2.at(-1)!;
      add({ id: thread, bot_id: b, key: null, title: s.title, created_at: list2[0].started_at, archived_at: s.status === "closed" ? s.status_since : null, conversation: `${b}:${thread}` });
    }
  }
  const out: Chat[] = [];
  for (const [id, list] of conversations) {
    const mainId = mainBotOf(id);
    const order = participantOrder(list, mainId).filter((b) => mine.has(b));
    if (!order.length) continue;
    // Each bot's current thread: its newest open one, else its newest.
    const parts = order.map((botId) => {
      const own = list.filter((t) => t.bot_id === botId).sort((a, b) => a.created_at - b.created_at);
      const sessionsOf = (t: BotThread) => byThread.get(`${botId}\n${t.id}`) ?? [];
      const isOpen = (t: BotThread) => t.archived_at == null && sessionsOf(t).at(-1)?.status !== "closed";
      const thread = [...own].reverse().find(isOpen) ?? own.at(-1)!;
      return { bot: mine.get(botId)!, thread, session: sessionsOf(thread).at(-1) ?? null, all: own.flatMap(sessionsOf), open: own.some(isOpen) };
    });
    const archived = !parts.some((p) => p.open);
    const home = parts.find((p) => p.bot.id === mainId && (archived || p.open)) ?? parts.find((p) => archived || p.open) ?? parts[0];
    const current = parts.map((p) => p.session).filter((s): s is Session => !!s && (archived || s.status !== "closed"));
    const state = [...current].sort((a, b) => urgency(a) - urgency(b) || b.status_since - a.status_since)[0] ?? null;
    const all = parts.flatMap((p) => p.all);
    const newest = [...all].sort((a, b) => b.status_since - a.status_since);
    const answer = newest.find((s) => s.last_message)?.last_message ?? "";
    const routineThread = home.thread.key?.startsWith("routine:") ?? false;
    const routine = routineThread ? (home.session?.launch_spec.routine?.title ?? (home.session?.launch_spec.from?.kind === "routine" ? home.session.launch_spec.from.name : null) ?? home.thread.title) || "routine" : null;
    const status = state ? threadStatus(state) : archived ? { label: "Archived", tone: "idle" as const } : { label: "New", tone: "idle" as const };
    out.push({
      kind: "thread", id, pin: `conversation:${id}`, bot: home.bot, thread: home.thread.id, threads: list.map((t) => `${t.bot_id}\n${t.id}`), title: chatTitle(home.thread, [...home.all].sort((x, y) => x.started_at - y.started_at)[0] ?? null, home.session),
      participants: parts.map((p) => p.bot), session: state, sessionIds: all.map((s) => s.id), ...status,
      unseen: current.some(unseenTurn), archived, routine,
      line: isNoUpdateText(answer) ? "No update (a routine with nothing to report)" : answer,
      last: Math.max(newest[0]?.status_since ?? 0, ...list.map((t) => t.archived_at ?? t.created_at)),
      section: sectionOf(current, archived),
    });
  }
  for (const c of channels ?? []) {
    if (c.workspace_id !== workspaceId) continue;
    const ask = asks?.get(c.id) ?? 0;
    out.push({
      kind: "channel", id: c.id, pin: `channel:${c.id}`, channel: c, title: c.kind === "dm" ? c.title : c.name, members: c.members.map((m) => m.bot_id),
      unread: c.unread, asks: ask, archived: c.archived, line: channelLine(c), last: c.last_at ?? c.created_at,
      section: c.archived ? "Archived" : c.unread > 0 || ask > 0 ? "Unread" : "Channels",
    });
  }
  return out.sort((a, b) => b.last - a.last || a.title.localeCompare(b.title));
}

function channelLine(c: Channel): string {
  if (!c.last_text) return c.purpose || "No message yet";
  const by = c.last_by === "you" ? "You" : c.last_by;
  return by ? `${by}: ${c.last_text}` : c.last_text;
}

/** The bots a chat involves: a conversation's participants, a channel's members. */
export const chatBots = (c: Chat): string[] => (c.kind === "thread" ? c.participants.map((b) => b.id) : c.members);

/** The lower-cased text the search box matches: title, last line, bots, a channel's purpose. */
export function chatSearchText(c: Chat): string {
  const names = c.kind === "thread" ? c.participants.map((b) => b.settings.name) : c.channel.members.map((m) => m.name);
  return [c.title, c.line, ...names, c.kind === "channel" ? `${c.channel.title} ${c.channel.purpose}` : c.routine ?? ""].join("\n").toLocaleLowerCase();
}

/** The chats a search and a bot filter keep. Every selected bot must take part in the chat (a conversation's bots,
 * a channel's members). `matched`: sessions whose messages match the search (`/api/sessions/search`), and
 * `matchedChannels`: channels whose messages match it (`/api/chat-search`), when known. */
export function filterChats<T extends Chat>(chats: readonly T[], { needle = "", bots = [], matched = null, matchedChannels = null }: { needle?: string; bots?: readonly string[]; matched?: ReadonlySet<string> | null; matchedChannels?: ReadonlySet<string> | null }): T[] {
  const q = needle.trim().toLocaleLowerCase();
  return chats.filter((c) => {
    if (bots.length) {
      const involved = new Set(chatBots(c));
      if (!bots.every((b) => involved.has(b))) return false;
    }
    if (!q) return true;
    if (chatSearchText(c).includes(q)) return true;
    return c.kind === "thread" ? !!matched && c.sessionIds.some((id) => matched.has(id)) : !!matchedChannels && matchedChannels.has(c.id);
  });
}

/** The chats by section, in the sections' order, each newest first; empty sections left out. `pinned` (the phone,
 * which has no sidebar): those chats go first, in a Pinned section of their own, in the order given. */
export function chatSections<T extends Chat>(chats: readonly T[], pinned?: readonly string[]): { section: ChatSection; chats: T[] }[] {
  const first = pinned?.length ? pinned.map((p) => chats.find((c) => c.pin === p && !c.archived)).filter((c): c is T => !!c) : [];
  const rest = chats.filter((c) => !first.includes(c));
  return CHAT_SECTIONS.map((section) => ({ section, chats: section === "Pinned" ? first : rest.filter((c) => c.section === section) })).filter((g) => g.chats.length > 0);
}

/** The chat a bot page shows (its bot and thread), or a channel page (its ID). */
export const chatOf = <T extends Chat>(chats: readonly T[], at: { bot: string; thread: string } | { channel: string }): T | undefined =>
  "channel" in at ? chats.find((c) => c.kind === "channel" && c.id === at.channel) : chats.find((c) => c.kind === "thread" && c.threads.includes(`${at.bot}\n${at.thread}`));

/** Where a chat opens. */
export const chatPath = (c: Chat) => (c.kind === "thread" ? `/bots/${c.bot.id}?thread=${encodeURIComponent(c.thread)}` : `/channels/${c.id}`);

/** A chat that needs you: a question, an unseen turn, or a channel message that asks you. */
export const chatNeedsYou = (c: Chat) => c.kind === "thread" ? !c.archived && (c.section === "Needs you" || c.section === "Unread") : !c.archived && c.asks > 0;

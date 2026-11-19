// Active threads (the phone's Chats tab, the Bots page and the sidebar): the open threads of every bot of the workspace
// in one list, newest update first. Active = not archived, and live or waiting for you (working, a question or an
// unseen turn, sleeping, quota paused, failed or interrupted) or updated in the last 24 hours. Pure, so it is tested
// without the app. A conversation several bots share (`BotThread.conversation`) is one row: its main bot's thread,
// every bot's mark, and the most urgent state of its threads (Question > Failed > Working > Sleeping > Done).

import type { Bot } from "../gen/Bot";
import type { BotThread } from "../gen/BotThread";
import type { Session } from "../gen/Session";
import { threadOf } from "./botThread.ts";
import { mainBotOf } from "./conversations.ts";
import { isQuotaPaused, isRunning, isSleeping, needsYou } from "./sessionStatus.ts";

/** How recent a thread with nothing live must be to count as active. */
export const ACTIVE_WINDOW_MS = 24 * 60 * 60 * 1000;
/** Rows shown before Show all. */
export const ACTIVE_SHOWN = 6;

export type ThreadTone = "running" | "needs" | "done" | "failed" | "idle";
export type ActiveThread = {
  bot: Bot; thread: string; title: string; session: Session; last: number; attention: boolean; label: string; tone: ThreadTone;
  /** Its conversation (`BotThread.conversation`). */
  conversation: string;
  /** The bots of the conversation that have an active thread in it, the main bot first (one for a bot's own thread). */
  bots: Bot[];
};

const failed = (s: Session) => s.status === "error" || s.status === "interrupted";

/** Live or waiting for you: never hidden behind the cap by newer finished threads. */
export const isAttention = (s: Session) => isRunning(s) || needsYou(s) || isSleeping(s) || isQuotaPaused(s) || failed(s);

/** A short status, in the words and colors of the session status pill (`statusInfo`). */
export function threadStatus(s: Session): { label: string; tone: ThreadTone } {
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

/** How urgent a thread's state is, lowest first: Question (or quota paused) > Failed > Working > Sleeping > Done. */
export function urgency(r: { label: string; tone: ThreadTone }): number {
  if (r.tone === "needs") return 0;
  if (r.tone === "failed") return 1;
  if (r.tone === "running") return 2;
  if (r.label === "Sleeping") return 3;
  if (r.tone === "done") return 4;
  return 5;
}

/** Rows of one conversation become one: the main bot's thread (else the most urgent one), its title, the most urgent
 * state, the newest update and every bot's mark, the main bot first. */
export function oneRowPerConversation(rows: ActiveThread[]): ActiveThread[] {
  const groups = new Map<string, ActiveThread[]>();
  for (const r of rows) groups.set(r.conversation, [...(groups.get(r.conversation) ?? []), r]);
  const out: ActiveThread[] = [];
  for (const [conversation, members] of groups) {
    if (members.length === 1) { out.push(members[0]); continue; }
    const main = mainBotOf(conversation);
    const urgent = [...members].sort((a, b) => urgency(a) - urgency(b) || b.last - a.last)[0];
    const home = members.find((m) => m.bot.id === main) ?? urgent;
    const bots = [...members].sort((a, b) => Number(b.bot.id === main) - Number(a.bot.id === main) || a.bot.settings.name.localeCompare(b.bot.settings.name)).map((m) => m.bot);
    out.push({ ...home, session: urgent.session, label: urgent.label, tone: urgent.tone, attention: members.some((m) => m.attention), last: Math.max(...members.map((m) => m.last)), bots });
  }
  return out;
}

/** The active threads of the workspace's bots (one row per conversation), newest update first. `threads` holds each bot's open threads (Home's
 * list, for their titles) when loaded: a thread missing from a loaded list is archived. */
export function activeThreads(bots: Bot[], sessions: Session[], threads: Map<string, BotThread[] | undefined>, workspaceId: string | null | undefined, now: number): ActiveThread[] {
  const mine = new Map(bots.filter((b) => b.workspace_id === workspaceId).map((b) => [b.id, b]));
  // Each thread's newest session.
  const current = new Map<string, Session>();
  for (const s of [...sessions].sort((a, b) => a.started_at - b.started_at)) {
    const bot = s.launch_spec.bot?.id;
    if (bot && mine.has(bot)) current.set(`${bot}\n${threadOf(s)}`, s);
  }
  const rows: ActiveThread[] = [];
  for (const s of current.values()) {
    if (s.status === "closed") continue;
    const bot = mine.get(s.launch_spec.bot!.id)!;
    const thread = threadOf(s);
    const list = threads.get(bot.id);
    const record = list?.find((t) => t.id === thread);
    if (list && !record) continue;
    const attention = isAttention(s);
    if (!attention && now - s.status_since > ACTIVE_WINDOW_MS) continue;
    rows.push({ bot, thread, title: record?.title || s.title, session: s, last: s.status_since, attention, ...threadStatus(s), conversation: record?.conversation ?? `${bot.id}:${thread}`, bots: [bot] });
  }
  return oneRowPerConversation(rows).sort((a, b) => b.last - a.last || a.bot.settings.name.localeCompare(b.bot.settings.name));
}

/** At most `max` rows in their order, keeping the threads that need attention before newer finished ones. */
export function capThreads(rows: ActiveThread[], max = ACTIVE_SHOWN): ActiveThread[] {
  if (rows.length <= max) return rows;
  const attention = rows.filter((r) => r.attention).slice(0, max);
  const keep = new Set([...attention, ...rows.filter((r) => !r.attention).slice(0, max - attention.length)]);
  return rows.filter((r) => keep.has(r));
}
